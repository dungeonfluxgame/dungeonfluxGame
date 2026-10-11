//! Player-owned presentation of the native dialogue ticket and exact confirmation.
use super::*;
use web_sys::HtmlTextAreaElement;

#[derive(Default)]
pub(super) struct DialogueRecovery {
    channel: Option<rpc::dialogue_service_client::DialogueServiceClient<BrowserChannel>>,
    editor: Option<HtmlTextAreaElement>,
    context: Option<HtmlSelectElement>,
    offer: Option<HtmlSelectElement>,
    offers: Vec<rpc::GameplayActionOffer>,
    ticket: Option<(rpc::RawDialogueRequest, rpc::DialogueResponse)>,
    confirmation: Option<rpc::ConfirmDialogueRequest>,
    active: Option<(Rc<()>, AbortHandle)>,
    message: String,
    restore_focus: bool,
    selection: Option<(u32, u32)>,
}

impl DialogueRecovery {
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
        self.retire_transport();
        self.confirmation = None;
        self.message =
            "The input scope changed. Your text remains editable; choose a current offer."
                .to_owned();
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

pub(super) fn mount(
    client: &Rc<RefCell<Client>>,
    parent: &Element,
    offers: &[rpc::GameplayActionOffer],
) -> Result<(), JsValue> {
    if client.borrow().role != "player" {
        return Ok(());
    }
    let (document, editor, context, selector, focused, selection) = {
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
        let context = match recovery.context.as_ref() {
            Some(context) => context.clone(),
            None => {
                let context = document
                    .create_element("select")?
                    .dyn_into::<HtmlSelectElement>()?;
                context.set_attribute("aria-label", "Input context")?;
                for (kind, label) in [
                    (rpc::DialogueContext::Unspecified, "Uncertain intention"),
                    (rpc::DialogueContext::Question, "Question"),
                    (rpc::DialogueContext::Social, "Conversation"),
                    (rpc::DialogueContext::PlanOnly, "Plan only"),
                    (rpc::DialogueContext::Meta, "About the game"),
                    (rpc::DialogueContext::Joke, "Joke"),
                ] {
                    let option = node(&document, context.as_ref(), "option", "", label)?;
                    option.set_attribute("value", &(kind as i32).to_string())?;
                }
                recovery.context = Some(context.clone());
                context
            }
        };
        let selector = match recovery.offer.as_ref() {
            Some(selector) => selector.clone(),
            None => {
                let selector = document
                    .create_element("select")?
                    .dyn_into::<HtmlSelectElement>()?;
                selector.set_attribute("aria-label", "Separate offered choice")?;
                recovery.offer = Some(selector.clone());
                selector
            }
        };
        let focused = recovery.restore_focus;
        let selection = recovery.selection;
        (document, editor, context, selector, focused, selection)
    };
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
        "Text is a draft, never an automatic action or check. A source-grounded ruling is unavailable here; decline or choose a supported offered action.",
    )?;
    panel.append_child(&editor)?;
    panel.append_child(&context)?;
    let previous = selector.value();
    selector.set_text_content(None);
    let none = node(
        &document,
        selector.as_ref(),
        "option",
        "",
        "No action selected",
    )?;
    none.set_attribute("value", "")?;
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
        let option = node(&document, selector.as_ref(), "option", "", &offer.label)?;
        option.set_attribute("value", &offer.offer_id)?;
        current.push(offer.clone());
    }
    if current.iter().any(|offer| offer.offer_id == previous) {
        selector.set_value(&previous);
    }
    panel.append_child(&selector)?;
    {
        let mut state = client.borrow_mut();
        let revision = state.revision;
        let recovery = &mut state.dialogue;
        if recovery.confirmation.is_none()
            && recovery.ticket.as_ref().is_some_and(|(input, _)| {
                input.observed_revision != revision
                    || !current.iter().any(|offer| offer.offer_id == input.offer_id)
            })
        {
            recovery.ticket = None;
            recovery.message =
                "The offer changed. Your text remains editable; request current clarification."
                    .to_owned();
        }
        recovery.offers = current;
    }
    let controls = node(&document, &panel, "div", "dialogue-controls", "")?;
    let raw = button(client, &controls, "Ask for clarification", submit)?;
    let cancel = button(client, &controls, "Decline this choice", decline)?;
    let confirm = button(client, &controls, "Confirm offered choice", confirm)?;
    let retry = button(client, &controls, "Retry the same confirmation", retry)?;
    let state = client.borrow();
    let recovery = &state.dialogue;
    raw.set_attribute("data-dialogue", "submit")?;
    cancel.set_attribute("data-dialogue", "cancel")?;
    confirm.set_attribute("data-dialogue", "confirm")?;
    retry.set_attribute("data-dialogue", "retry")?;
    for (element, disabled) in [
        (
            &raw,
            state.busy || recovery.channel.is_none() || recovery.confirmation.is_some(),
        ),
        (
            &cancel,
            state.busy
                || recovery.channel.is_none()
                || recovery.ticket.is_none()
                || recovery.confirmation.is_some(),
        ),
        (
            &confirm,
            state.busy
                || recovery.channel.is_none()
                || recovery.ticket.is_none()
                || recovery.confirmation.is_some(),
        ),
        (
            &retry,
            state.busy || recovery.channel.is_none() || recovery.confirmation.is_none(),
        ),
    ] {
        if disabled {
            element.set_attribute("disabled", "")?;
        }
    }
    context
        .set_disabled(state.busy || recovery.ticket.is_some() || recovery.confirmation.is_some());
    selector
        .set_disabled(state.busy || recovery.ticket.is_some() || recovery.confirmation.is_some());
    let message = node(
        &document,
        &panel,
        "p",
        "dialogue-feedback",
        &recovery.message,
    )?;
    message.set_attribute("role", "status")?;
    if focused {
        editor.focus()?;
        if let Some((start, end)) = selection {
            editor.set_selection_range(start, end)?;
        }
    }
    Ok(())
}

fn submit(client: Rc<RefCell<Client>>) {
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
        let context = state
            .dialogue
            .context
            .as_ref()
            .ok_or("Input context absent")?
            .value()
            .parse::<i32>()
            .map_err(|_| "Input context invalid")?;
        let context =
            rpc::DialogueContext::try_from(context).map_err(|_| "Input context invalid")?;
        let offer = state
            .dialogue
            .offer
            .as_ref()
            .ok_or("Offer control absent")?
            .value();
        if !offer.is_empty()
            && !state
                .dialogue
                .offers
                .iter()
                .any(|item| item.offer_id == offer)
        {
            return Err("Choose a current offered action.".to_owned());
        }
        Ok(rpc::RawDialogueRequest {
            session_id: state.session.clone(),
            run_id: state.run.clone(),
            observed_revision: state.revision,
            input_id: Some(operation()?),
            text,
            context: context as i32,
            finality: rpc::DialogueFinality::Final as i32,
            offer_id: offer,
        })
    })();
    match prepared {
        Ok(body) => dispatch(client, Call::Raw(body)),
        Err(message) => show(&client, &message),
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
        if matches!(&call, Call::Raw(_)) {
            state.dialogue.ticket = None;
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
            state.dialogue.confirmation = Some(body.clone());
        }
        let instance = Rc::new(());
        let (abort, registration) = AbortHandle::new_pair();
        if let Some((_, old)) = state.dialogue.active.replace((instance.clone(), abort)) {
            old.abort();
        }
        state.busy = true;
        state.dialogue.message = "Waiting for the native dialogue owner…".to_owned();
        Ok((channel, credential, generation, instance, registration))
    })();
    let (mut channel, credential, generation, instance, registration) = match prepared {
        Ok(value) => value,
        Err(message) => {
            show(&client, &message);
            return;
        }
    };
    let weak = Rc::downgrade(&client);
    let _ = redraw_current(&client);
    wasm_bindgen_futures::spawn_local(async move {
        let run =
            async {
                match &call {
                    Call::Raw(body) => channel
                        .submit(request(body.clone(), &credential).map_err(|_| {
                            tonic::Status::unauthenticated("input binding unavailable")
                        })?)
                        .await
                        .map(|reply| (Some(reply.into_inner()), None)),
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
            state.busy = false;
            let message = match outcome {
                Ok(Ok((Some(reply), None))) => match &call {
                    Call::Raw(input)
                        if reply.input_id == input.input_id
                            && reply.revision == input.observed_revision
                            && state.revision == input.observed_revision =>
                    {
                        match rpc::DialogueDisposition::try_from(reply.disposition) {
                            Ok(rpc::DialogueDisposition::Clarify)
                                if reply.confirmation_required
                                    && reply.confirmation_token.len() == 32
                                    && !input.offer_id.is_empty() =>
                            {
                                state.dialogue.ticket = Some((input.clone(), reply));
                                "Clarification required. Decline, or explicitly confirm the selected offered action.".to_owned()
                            }
                            Ok(kind)
                                if !reply.confirmation_required
                                    && reply.confirmation_token.is_empty()
                                    && kind != rpc::DialogueDisposition::Action
                                    && kind != rpc::DialogueDisposition::Unspecified =>
                            {
                                state.dialogue.ticket = None;
                                match kind {
                                    rpc::DialogueDisposition::Clarify => "Clarification required. Choose a supported offer or edit your draft.",
                                    rpc::DialogueDisposition::Rejected => "The native owner declined this input. Your draft remains editable.",
                                    _ => "The native owner classified this as conversation, not an action.",
                                }.to_owned()
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
                                "The offer or scope changed. Your text remains editable."
                            }
                            tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => {
                                "Dialogue authorization is unavailable. Reconnect."
                            }
                            tonic::Code::InvalidArgument | tonic::Code::ResourceExhausted => {
                                "The native owner refused this input. Edit the bounded draft."
                            }
                            _ => "Dialogue is unavailable. Your draft remains editable.",
                        }
                        .to_owned()
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
}
