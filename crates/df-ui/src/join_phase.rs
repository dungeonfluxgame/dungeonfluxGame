use std::{collections::BTreeSet, fmt};

use crate::{CampaignLimits, CampaignValidationError, CampaignView, MAX_DRAFT_UTF16_UNITS};

/// Presentation scope assigned by the shell; never a membership or credential.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JoinStamp {
    pub generation: u64,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JoinStage {
    Title,
    Invitation,
    Lobby,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JoinFeedback<'a> {
    None,
    Pending(&'a str),
    Rejected(&'a str),
}

/// Already permitted action offered by the role composition. The opaque key is
/// returned unchanged; the shell maps it to its existing typed command contract.
pub struct LobbyOffer<'a> {
    pub key: &'a str,
    pub label: &'a str,
    pub enabled: bool,
    pub pending: bool,
}

pub struct LobbyParticipant<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub sigil: &'a str,
    pub readiness: &'a str,
    pub presence: &'a str,
}

pub struct JoinRoom<'a> {
    pub label: &'a str,
    pub code: &'a str,
    pub detail: &'a str,
}

/// Audience-safe, localized composition, not a second Join RPC schema. The
/// caller supplies connectivity separately from admission/readiness. Public join
/// URL and QR rendering stay with the existing Display-I04 component, which may
/// mount in pairing_slot(). No local URL, QR, auth or connection inference occurs.
pub struct JoinPhaseView<'a> {
    pub stamp: JoinStamp,
    pub stage: JoinStage,
    pub campaign: CampaignView<'a>,
    pub panel_heading: &'a str,
    pub panel_description: &'a str,
    pub invitation_label: &'a str,
    pub name_label: &'a str,
    pub join_label: &'a str,
    pub join_enabled: bool,
    pub feedback: JoinFeedback<'a>,
    pub room: Option<JoinRoom<'a>>,
    pub roster_heading: &'a str,
    pub empty_roster: &'a str,
    pub participants: &'a [LobbyParticipant<'a>],
    pub lobby_offer: Option<LobbyOffer<'a>>,
    pub pairing_fallback: &'a str,
}

/// Input intent only. Emitting this does not join, ready, or start a session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JoinPhaseIntent {
    Join {
        stamp: JoinStamp,
        invitation: String,
        player_name: String,
    },
    LobbyAction {
        stamp: JoinStamp,
        offer_key: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JoinUpdate {
    Applied,
    StaleGeneration,
    StaleSequence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JoinValidationError {
    Campaign(CampaignValidationError),
    EmptyText,
    ResourceLimit,
    DuplicateParticipant,
    InvalidStage,
    DraftTooLong,
    Disposed,
    GenerationNotAdvanced,
}

impl fmt::Display for JoinValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Campaign(_) => "invalid campaign presentation",
            Self::EmptyText => "join presentation text is empty",
            Self::ResourceLimit => "join presentation exceeds owner limits",
            Self::DuplicateParticipant => "lobby participant keys repeat",
            Self::InvalidStage => "lobby offer is outside the lobby",
            Self::DraftTooLong => "join draft exceeds presentation length limit",
            Self::Disposed => "join presentation is disposed",
            Self::GenerationNotAdvanced => "replacement join generation must advance",
        })
    }
}

impl std::error::Error for JoinValidationError {}

impl JoinPhaseView<'_> {
    pub fn validate(&self, limits: CampaignLimits) -> Result<(), JoinValidationError> {
        self.campaign
            .validate(limits)
            .map_err(JoinValidationError::Campaign)?;
        let text = |value: &str| {
            if value.trim().is_empty() {
                Err(JoinValidationError::EmptyText)
            } else if value.len() > limits.max_text_bytes {
                Err(JoinValidationError::ResourceLimit)
            } else {
                Ok(())
            }
        };
        for value in [
            self.panel_heading,
            self.panel_description,
            self.invitation_label,
            self.name_label,
            self.join_label,
            self.roster_heading,
            self.empty_roster,
            self.pairing_fallback,
        ] {
            text(value)?;
        }
        if let JoinFeedback::Pending(message) | JoinFeedback::Rejected(message) = self.feedback {
            text(message)?;
        }
        if let Some(room) = &self.room {
            for value in [room.label, room.code] {
                text(value)?;
            }
            // A room with no address to share (a game on this computer only) has no detail line.
            if !room.detail.is_empty() {
                text(room.detail)?;
            }
        }
        if self.participants.len() > limits.max_members {
            return Err(JoinValidationError::ResourceLimit);
        }
        let mut keys = BTreeSet::new();
        for participant in self.participants {
            for value in [
                participant.key,
                participant.name,
                participant.sigil,
                participant.readiness,
                participant.presence,
            ] {
                text(value)?;
            }
            if !keys.insert(participant.key) {
                return Err(JoinValidationError::DuplicateParticipant);
            }
        }
        if let Some(offer) = &self.lobby_offer {
            if self.stage != JoinStage::Lobby {
                return Err(JoinValidationError::InvalidStage);
            }
            text(offer.key)?;
            text(offer.label)?;
        }
        Ok(())
    }
}

/// Checks storage bounds only, without interpreting invitation grammar or names.
pub fn validate_join_drafts(
    invitation: &str,
    player_name: &str,
) -> Result<(), JoinValidationError> {
    validate_draft(invitation)?;
    validate_draft(player_name)
}

fn validate_draft(value: &str) -> Result<(), JoinValidationError> {
    if value.encode_utf16().take(MAX_DRAFT_UTF16_UNITS + 1).count() > MAX_DRAFT_UTF16_UNITS {
        Err(JoinValidationError::DraftTooLong)
    } else {
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
struct JoinScope {
    stamp: JoinStamp,
    stage: JoinStage,
    join_enabled: bool,
    offer: Option<String>,
    disposed: bool,
    suspended: bool,
}

#[cfg(any(target_arch = "wasm32", test))]
impl JoinScope {
    fn new(view: &JoinPhaseView<'_>) -> Self {
        let mut scope = Self {
            stamp: view.stamp,
            stage: view.stage,
            join_enabled: false,
            offer: None,
            disposed: false,
            suspended: false,
        };
        scope.commit(view);
        scope
    }

    fn check(&self, stamp: JoinStamp) -> Result<JoinUpdate, JoinValidationError> {
        if self.disposed {
            return Err(JoinValidationError::Disposed);
        }
        if stamp.generation != self.stamp.generation {
            return Ok(JoinUpdate::StaleGeneration);
        }
        if stamp.sequence < self.stamp.sequence
            || (stamp.sequence == self.stamp.sequence && !self.suspended)
        {
            return Ok(JoinUpdate::StaleSequence);
        }
        Ok(JoinUpdate::Applied)
    }

    fn begin_replacement(&mut self, stamp: JoinStamp) -> Result<(), JoinValidationError> {
        if self.disposed {
            return Err(JoinValidationError::Disposed);
        }
        if stamp.generation <= self.stamp.generation {
            return Err(JoinValidationError::GenerationNotAdvanced);
        }
        // Ownership retirement is irreversible even if later DOM reconciliation
        // fails. The attempted stamp may be retried while suspended.
        self.begin_render(stamp);
        Ok(())
    }

    fn begin_render(&mut self, stamp: JoinStamp) {
        // Every valid attempt advances the watermark before fallible rendering.
        // Equality recovery belongs only to this attempted snapshot, never the
        // previously committed snapshot or a superseded failed attempt.
        self.stamp = stamp;
        self.join_enabled = false;
        self.offer = None;
        self.suspended = true;
    }

    fn commit(&mut self, view: &JoinPhaseView<'_>) {
        self.stamp = view.stamp;
        self.stage = view.stage;
        self.join_enabled = view.join_enabled
            && view.stage != JoinStage::Lobby
            && !matches!(view.feedback, JoinFeedback::Pending(_));
        self.offer = view
            .lobby_offer
            .as_ref()
            .filter(|offer| offer.enabled && !offer.pending)
            .map(|offer| offer.key.to_owned());
        self.suspended = false;
    }

    fn join_intent(&self, invitation: String, player_name: String) -> Option<JoinPhaseIntent> {
        if self.disposed || self.suspended || !self.join_enabled {
            return None;
        }
        Some(JoinPhaseIntent::Join {
            stamp: self.stamp,
            invitation,
            player_name,
        })
    }

    fn lobby_intent(&self) -> Option<JoinPhaseIntent> {
        if self.disposed || self.suspended || self.stage != JoinStage::Lobby {
            return None;
        }
        self.offer.as_ref().map(|key| JoinPhaseIntent::LobbyAction {
            stamp: self.stamp,
            offer_key: key.clone(),
        })
    }

    fn dispose(&mut self) {
        self.disposed = true;
        self.join_enabled = false;
        self.offer = None;
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::{
        ActionView, CampaignError, CampaignSurface, ControlError, ControlledAction,
        ControlledTextInput, DraftUpdate, InputFeedback, TextInputView, UiError,
    };
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
    use wasm_bindgen::JsCast;
    use web_sys::{Document, Element, HtmlElement};

    #[derive(Debug)]
    pub enum JoinPhaseError {
        InvalidView(JoinValidationError),
        Campaign(CampaignError),
        Control(ControlError),
        Dom(UiError),
        CallbackAlreadyRegistered,
    }
    impl fmt::Display for JoinPhaseError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidView(error) => fmt::Display::fmt(error, formatter),
                Self::Campaign(error) => fmt::Display::fmt(error, formatter),
                Self::Control(error) => fmt::Display::fmt(error, formatter),
                Self::Dom(error) => fmt::Display::fmt(error, formatter),
                Self::CallbackAlreadyRegistered => {
                    formatter.write_str("join intent callback already registered")
                }
            }
        }
    }
    impl std::error::Error for JoinPhaseError {}
    impl From<JoinValidationError> for JoinPhaseError {
        fn from(error: JoinValidationError) -> Self {
            Self::InvalidView(error)
        }
    }
    impl From<CampaignError> for JoinPhaseError {
        fn from(error: CampaignError) -> Self {
            Self::Campaign(error)
        }
    }
    impl From<ControlError> for JoinPhaseError {
        fn from(error: ControlError) -> Self {
            Self::Control(error)
        }
    }
    impl From<wasm_bindgen::JsValue> for JoinPhaseError {
        fn from(error: wasm_bindgen::JsValue) -> Self {
            Self::Dom(UiError::Browser(error))
        }
    }

    type IntentCallback = Box<dyn FnMut(JoinPhaseIntent)>;
    struct ParticipantNodes {
        root: Element,
        sigil: Element,
        name: Element,
        readiness: Element,
        presence: Element,
    }

    /// Persistent title, invitation and lobby component. Owns four bounded native
    /// control listeners; has no network, media jobs, timers or gameplay authority.
    /// Dispose the externally mounted Display-I04 pairing component first.
    pub struct JoinPhaseSurface {
        document: Document,
        campaign: CampaignSurface,
        heading: Element,
        description: Element,
        form: Element,
        invitation: Rc<ControlledTextInput>,
        name: Rc<ControlledTextInput>,
        join: ControlledAction,
        lobby: Element,
        room: Element,
        room_label: Element,
        room_code: Element,
        room_detail: Element,
        roster_heading: Element,
        roster: Element,
        empty: Element,
        lobby_action: ControlledAction,
        feedback: Element,
        pairing_slot: Element,
        participants: RefCell<BTreeMap<String, ParticipantNodes>>,
        scope: Rc<RefCell<JoinScope>>,
        callback: Rc<RefCell<Option<IntentCallback>>>,
        limits: CampaignLimits,
    }

    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
    ) -> Result<Element, JoinPhaseError> {
        let node = document.create_element(tag)?;
        node.set_class_name(class);
        parent.append_child(&node)?;
        Ok(node)
    }
    fn hidden(node: &Element, hide: bool) -> Result<(), JoinPhaseError> {
        if hide {
            node.set_attribute("hidden", "")?;
        } else {
            node.remove_attribute("hidden")?;
        }
        Ok(())
    }
    fn feedback(value: JoinFeedback<'_>) -> InputFeedback<'_> {
        match value {
            JoinFeedback::None => InputFeedback::None,
            JoinFeedback::Pending(message) => InputFeedback::Pending(message),
            JoinFeedback::Rejected(message) => InputFeedback::Rejected(message),
        }
    }
    fn emit(
        callback: &Rc<RefCell<Option<IntentCallback>>>,
        scope: &Rc<RefCell<JoinScope>>,
        intent: Option<JoinPhaseIntent>,
    ) {
        let Some(intent) = intent else {
            return;
        };
        let owned = callback.borrow_mut().take();
        if let Some(mut owned) = owned {
            owned(intent);
            if !scope.borrow().disposed {
                *callback.borrow_mut() = Some(owned);
            }
        }
    }
    impl JoinPhaseSurface {
        /// The unique identifier prefix follows ControlledTextInput's ID grammar.
        /// Initial drafts are supplied by the current shell owner, never persisted.
        pub fn create(
            document: &Document,
            identifier: &str,
            view: &JoinPhaseView<'_>,
            invitation_draft: &str,
            name_draft: &str,
            limits: CampaignLimits,
        ) -> Result<Self, JoinPhaseError> {
            view.validate(limits)?;
            validate_join_drafts(invitation_draft, name_draft)?;
            let campaign = CampaignSurface::create(document, &view.campaign, limits)?;
            campaign.root().set_attribute("data-join-phase", "")?;
            child(document, campaign.root(), "style", "")?
                .set_text_content(Some(crate::join_phase_theme::STYLES));
            let stage = campaign
                .root()
                .query_selector(".stage")?
                .ok_or(JoinPhaseError::Dom(UiError::WrongElementType))?;
            let panel = child(document, &stage, "section", "join-panel")?;
            let heading = child(document, &panel, "h2", "join-heading")?;
            heading.set_attribute("tabindex", "-1")?;
            let description = child(document, &panel, "p", "join-description")?;
            let form = child(document, &panel, "div", "join-fields")?;
            let invitation = Rc::new(ControlledTextInput::create(
                document,
                &format!("{identifier}-invitation"),
                TextInputView {
                    label: view.invitation_label,
                    enabled: view.join_enabled,
                    feedback: feedback(view.feedback),
                },
                invitation_draft,
            )?);
            invitation.input().set_attribute("autocomplete", "off")?;
            invitation.input().set_attribute("spellcheck", "false")?;
            invitation
                .input()
                .set_attribute("aria-label", view.invitation_label)?;
            let name = Rc::new(ControlledTextInput::create(
                document,
                &format!("{identifier}-name"),
                TextInputView {
                    label: view.name_label,
                    enabled: view.join_enabled,
                    feedback: InputFeedback::None,
                },
                name_draft,
            )?);
            name.input().set_attribute("autocomplete", "nickname")?;
            let join = ControlledAction::create(
                document,
                ActionView {
                    label: view.join_label,
                    enabled: false,
                    pending: false,
                },
            )?;
            form.append_child(invitation.root())?;
            form.append_child(name.root())?;
            form.append_child(join.element())?;
            let lobby = child(document, &panel, "div", "join-lobby")?;
            let room = child(document, &lobby, "div", "join-room")?;
            let room_label = child(document, &room, "div", "overline")?;
            let room_code = child(document, &room, "div", "join-room-code")?;
            let room_detail = child(document, &room, "p", "join-room-detail")?;
            let roster_heading = child(document, &lobby, "h3", "join-roster-heading")?;
            let roster = child(document, &lobby, "ul", "join-roster")?;
            let empty = child(document, &lobby, "p", "join-empty")?;
            let lobby_action = ControlledAction::create(
                document,
                ActionView {
                    label: view.join_label,
                    enabled: false,
                    pending: false,
                },
            )?;
            lobby.append_child(lobby_action.element())?;
            let feedback = child(document, &panel, "p", "join-status")?;
            feedback.set_attribute("role", "status")?;
            feedback.set_attribute("aria-live", "polite")?;
            let pairing_slot = child(document, &panel, "aside", "join-pairing")?;
            let fallback = child(document, &pairing_slot, "p", "join-pairing-fallback")?;
            fallback.set_text_content(Some(view.pairing_fallback));
            let surface = Self {
                document: document.clone(),
                campaign,
                heading,
                description,
                form,
                invitation,
                name,
                join,
                lobby,
                room,
                room_label,
                room_code,
                room_detail,
                roster_heading,
                roster,
                empty,
                lobby_action,
                feedback,
                pairing_slot,
                participants: RefCell::new(BTreeMap::new()),
                scope: Rc::new(RefCell::new(JoinScope::new(view))),
                callback: Rc::new(RefCell::new(None)),
                limits,
            };
            surface.bind_controls()?;
            surface.render(view, DraftUpdate::Preserve, DraftUpdate::Preserve)?;
            Ok(surface)
        }
        pub fn root(&self) -> &Element {
            self.campaign.root()
        }
        pub fn navigation(&self) -> &Element {
            self.campaign.navigation()
        }
        pub fn invitation(&self) -> &ControlledTextInput {
            &self.invitation
        }
        pub fn player_name(&self) -> &ControlledTextInput {
            &self.name
        }
        pub fn pairing_slot(&self) -> &Element {
            &self.pairing_slot
        }
        pub fn on_intent(
            &self,
            callback: impl FnMut(JoinPhaseIntent) + 'static,
        ) -> Result<(), JoinPhaseError> {
            if self.scope.borrow().disposed {
                return Err(JoinValidationError::Disposed.into());
            }
            // The two control listeners are installed once; replacing a callback is
            // forbidden even during reentrant intent delivery.
            if self.root().has_attribute("data-intent-bound") {
                return Err(JoinPhaseError::CallbackAlreadyRegistered);
            }
            self.root().set_attribute("data-intent-bound", "")?;
            *self.callback.borrow_mut() = Some(Box::new(callback));
            Ok(())
        }
        fn bind_controls(&self) -> Result<(), JoinPhaseError> {
            let scope = Rc::clone(&self.scope);
            let callback = Rc::clone(&self.callback);
            let invitation = Rc::clone(&self.invitation);
            let name = Rc::clone(&self.name);
            self.join.on_activate(move || {
                let intent = scope.borrow().join_intent(invitation.draft(), name.draft());
                emit(&callback, &scope, intent);
            })?;
            let scope = Rc::clone(&self.scope);
            let callback = Rc::clone(&self.callback);
            self.lobby_action.on_activate(move || {
                let intent = scope.borrow().lobby_intent();
                emit(&callback, &scope, intent);
            })?;
            Ok(())
        }
        /// Reconciles the same owner in place; drafts, input nodes, selection and
        /// focus survive accepted same-phase updates and server rejection. A DOM
        /// failure fences all older snapshots; retry the attempted stamp or newer.
        pub fn update(&self, view: &JoinPhaseView<'_>) -> Result<JoinUpdate, JoinPhaseError> {
            let outcome = self.scope.borrow().check(view.stamp)?;
            if outcome != JoinUpdate::Applied {
                return Ok(outcome);
            }
            view.validate(self.limits)?;
            self.scope.borrow_mut().begin_render(view.stamp);
            self.render(view, DraftUpdate::Preserve, DraftUpdate::Preserve)?;
            self.scope.borrow_mut().commit(view);
            Ok(JoinUpdate::Applied)
        }
        /// Explicit shell ownership replacement. Both obsolete drafts are cleared
        /// before rendering and the old generation is irreversibly fenced before
        /// any DOM work. A failed render remains suspended; update with the same
        /// replacement stamp or a newer sequence recovers the new owner. Caller
        /// must dispose/rebind an external Display-I04 slot component separately.
        pub fn replace_owner(&self, view: &JoinPhaseView<'_>) -> Result<(), JoinPhaseError> {
            if self.scope.borrow().disposed {
                return Err(JoinValidationError::Disposed.into());
            }
            if view.stamp.generation <= self.scope.borrow().stamp.generation {
                return Err(JoinValidationError::GenerationNotAdvanced.into());
            }
            view.validate(self.limits)?;
            self.scope.borrow_mut().begin_replacement(view.stamp)?;
            // Native properties cannot fail. Clear visible private values and
            // stop editing before attempting either control's fallible DOM update.
            for field in [&self.invitation, &self.name] {
                field.input().set_value("");
                field.input().set_read_only(true);
            }
            let mut failure = None;
            for (field, label) in [
                (&self.invitation, view.invitation_label),
                (&self.name, view.name_label),
            ] {
                if let Err(error) = field.update(
                    TextInputView {
                        label,
                        enabled: false,
                        feedback: InputFeedback::None,
                    },
                    DraftUpdate::Replace(""),
                ) {
                    failure.get_or_insert(JoinPhaseError::Control(error));
                }
            }
            if let Some(error) = failure {
                return Err(error);
            }
            self.render(view, DraftUpdate::Preserve, DraftUpdate::Preserve)?;
            self.scope.borrow_mut().commit(view);
            Ok(())
        }
        fn render(
            &self,
            view: &JoinPhaseView<'_>,
            invitation: DraftUpdate<'_>,
            name: DraftUpdate<'_>,
        ) -> Result<(), JoinPhaseError> {
            let focus_becomes_unavailable =
                self.document
                    .active_element()
                    .as_ref()
                    .is_some_and(|active| {
                        (view.stage == JoinStage::Lobby && self.form.contains(Some(active)))
                            || (view.stage != JoinStage::Lobby && self.lobby.contains(Some(active)))
                            || (self.lobby_action.element().is_same_node(Some(active))
                                && view
                                    .lobby_offer
                                    .as_ref()
                                    .is_none_or(|offer| !offer.enabled || offer.pending))
                            || (self.join.element().is_same_node(Some(active))
                                && (!view.join_enabled
                                    || matches!(view.feedback, JoinFeedback::Pending(_))))
                    });
            self.campaign.update(&view.campaign)?;
            // Restore before changing hidden/disabled so a later unrelated DOM
            // failure cannot strand focus in an unavailable subtree.
            if focus_becomes_unavailable {
                self.heading
                    .dyn_ref::<HtmlElement>()
                    .ok_or(JoinPhaseError::Dom(UiError::WrongElementType))?
                    .focus()?;
            }
            self.root().set_attribute(
                "data-join-stage",
                match view.stage {
                    JoinStage::Title => "title",
                    JoinStage::Invitation => "invitation",
                    JoinStage::Lobby => "lobby",
                },
            )?;
            self.root()
                .set_attribute("data-join-generation", &view.stamp.generation.to_string())?;
            self.root()
                .set_attribute("data-join-sequence", &view.stamp.sequence.to_string())?;
            self.heading.set_text_content(Some(view.panel_heading));
            self.description
                .set_text_content(Some(view.panel_description));
            let pending = matches!(view.feedback, JoinFeedback::Pending(_));
            let editable = view.join_enabled && view.stage != JoinStage::Lobby;
            self.invitation.update(
                TextInputView {
                    label: view.invitation_label,
                    enabled: editable,
                    feedback: feedback(view.feedback),
                },
                invitation,
            )?;
            self.invitation
                .input()
                .set_attribute("aria-label", view.invitation_label)?;
            self.name.update(
                TextInputView {
                    label: view.name_label,
                    enabled: editable && !pending,
                    feedback: InputFeedback::None,
                },
                name,
            )?;
            self.join.update(ActionView {
                label: view.join_label,
                enabled: editable,
                pending,
            })?;
            hidden(&self.form, view.stage == JoinStage::Lobby)?;
            hidden(&self.lobby, view.stage != JoinStage::Lobby)?;
            hidden(&self.room, view.room.is_none())?;
            if let Some(room) = &view.room {
                self.room_label.set_text_content(Some(room.label));
                self.room_code.set_text_content(Some(room.code));
                self.room_detail.set_text_content(Some(room.detail));
                hidden(&self.room_detail, room.detail.is_empty())?;
            } else {
                for node in [&self.room_label, &self.room_code, &self.room_detail] {
                    node.set_text_content(None);
                }
            }
            self.roster_heading
                .set_text_content(Some(view.roster_heading));
            self.empty.set_text_content(Some(view.empty_roster));
            hidden(&self.empty, !view.participants.is_empty())?;
            self.render_participants(view.participants)?;
            hidden(
                self.lobby_action.element().as_ref(),
                view.lobby_offer.is_none(),
            )?;
            self.lobby_action.update(match &view.lobby_offer {
                Some(offer) => ActionView {
                    label: offer.label,
                    enabled: offer.enabled,
                    pending: offer.pending,
                },
                None => ActionView {
                    label: view.join_label,
                    enabled: false,
                    pending: false,
                },
            })?;
            self.feedback.set_text_content(match view.feedback {
                JoinFeedback::None => None,
                JoinFeedback::Pending(message) | JoinFeedback::Rejected(message) => Some(message),
            });
            self.feedback.set_attribute(
                "data-feedback",
                match view.feedback {
                    JoinFeedback::None => "none",
                    JoinFeedback::Pending(_) => "pending",
                    JoinFeedback::Rejected(_) => "rejected",
                },
            )?;
            if let Some(fallback) = self.pairing_slot.query_selector(".join-pairing-fallback")? {
                fallback.set_text_content(Some(view.pairing_fallback));
            }
            Ok(())
        }
        fn render_participants(
            &self,
            participants: &[LobbyParticipant<'_>],
        ) -> Result<(), JoinPhaseError> {
            let mut nodes = self.participants.borrow_mut();
            let obsolete: Vec<_> = nodes
                .keys()
                .filter(|key| !participants.iter().any(|p| p.key == key.as_str()))
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(node) = nodes.remove(&key) {
                    self.roster.remove_child(&node.root)?;
                }
            }
            for participant in participants {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    nodes.entry(participant.key.to_owned())
                {
                    let root = child(&self.document, &self.roster, "li", "join-participant")?;
                    root.set_attribute("data-participant-key", participant.key)?;
                    let sigil = child(&self.document, &root, "span", "join-sigil")?;
                    sigil.set_attribute("aria-hidden", "true")?;
                    let content = child(&self.document, &root, "div", "join-participant-content")?;
                    let name = child(&self.document, &content, "div", "join-participant-name")?;
                    let presence = child(&self.document, &content, "div", "join-presence")?;
                    let readiness = child(&self.document, &root, "span", "join-readiness")?;
                    entry.insert(ParticipantNodes {
                        root,
                        sigil,
                        name,
                        presence,
                        readiness,
                    });
                }
                if let Some(node) = nodes.get(participant.key) {
                    node.name.set_text_content(Some(participant.name));
                    node.sigil.set_text_content(Some(participant.sigil));
                    node.presence.set_text_content(Some(participant.presence));
                    node.readiness.set_text_content(Some(participant.readiness));
                    self.roster.append_child(&node.root)?;
                }
            }
            Ok(())
        }
        /// Idempotent, terminal disposal attempts all owned cleanup even if one DOM
        /// removal fails; callbacks are fenced before any fallible cleanup.
        pub fn dispose(&self) -> Result<(), JoinPhaseError> {
            self.scope.borrow_mut().dispose();
            self.callback.borrow_mut().take();
            let mut failure = None;
            for result in [
                self.invitation.dispose(),
                self.name.dispose(),
                self.join.dispose(),
                self.lobby_action.dispose(),
            ] {
                if let Err(error) = result {
                    failure.get_or_insert(JoinPhaseError::Control(error));
                }
            }
            self.participants.borrow_mut().clear();
            if let Err(error) = self.campaign.dispose() {
                failure.get_or_insert(JoinPhaseError::Campaign(error));
            }
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }
    impl Drop for JoinPhaseSurface {
        fn drop(&mut self) {
            self.scope.borrow_mut().dispose();
            self.callback.borrow_mut().take();
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{JoinPhaseError, JoinPhaseSurface};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConceptScene;

    fn limits() -> CampaignLimits {
        CampaignLimits {
            max_members: 32,
            max_objectives: 16,
            max_text_bytes: 4096,
        }
    }
    fn view<'a>(participants: &'a [LobbyParticipant<'a>]) -> JoinPhaseView<'a> {
        JoinPhaseView {
            stamp: JoinStamp {
                generation: 7,
                sequence: 1,
            },
            stage: JoinStage::Invitation,
            campaign: CampaignView {
                scene: ConceptScene::Harbor,
                chapter: "An invitation",
                title: "The Drowned Lantern",
                description: "Gather your party",
                location: "Greyhaven",
                scene_label: "Harbor",
                narration: "The lantern is waiting",
                connection: "Offline",
                notice: "Test fixture",
                members: &[],
                objectives: &[],
            },
            panel_heading: "Join the story",
            panel_description: "Bring your invitation",
            invitation_label: "Invitation",
            name_label: "Player name",
            join_label: "Join",
            join_enabled: true,
            feedback: JoinFeedback::None,
            room: None,
            roster_heading: "Party",
            empty_roster: "Awaiting adventurers",
            participants,
            lobby_offer: None,
            pairing_fallback: "Public join information unavailable",
        }
    }

    #[test]
    fn a_room_with_no_address_to_share_still_validates_and_an_empty_code_does_not() {
        let mut current = view(&[]);
        current.stage = JoinStage::Lobby;
        current.room = Some(JoinRoom {
            label: "Room code",
            code: "ABCD",
            detail: "",
        });
        assert_eq!(current.validate(limits()), Ok(()));
        current.room = Some(JoinRoom {
            label: "Room code",
            code: "",
            detail: "",
        });
        assert_eq!(
            current.validate(limits()),
            Err(JoinValidationError::EmptyText)
        );
    }

    #[test]
    fn stale_generation_and_sequence_cannot_replace_current_offer() {
        let mut current = view(&[]);
        current.stage = JoinStage::Lobby;
        current.lobby_offer = Some(LobbyOffer {
            key: "ready-current",
            label: "Ready",
            enabled: true,
            pending: false,
        });
        let scope = JoinScope::new(&current);
        assert_eq!(
            scope.check(JoinStamp {
                generation: 6,
                sequence: 500
            }),
            Ok(JoinUpdate::StaleGeneration)
        );
        assert_eq!(scope.check(current.stamp), Ok(JoinUpdate::StaleSequence));
        assert_eq!(
            scope.check(JoinStamp {
                generation: 7,
                sequence: 2
            }),
            Ok(JoinUpdate::Applied)
        );
        assert_eq!(
            scope.lobby_intent(),
            Some(JoinPhaseIntent::LobbyAction {
                stamp: current.stamp,
                offer_key: "ready-current".into(),
            })
        );
    }

    #[test]
    fn pending_rejection_and_disconnect_are_independent_of_membership() {
        let mut current = view(&[]);
        current.feedback = JoinFeedback::Pending("Awaiting response");
        let mut scope = JoinScope::new(&current);
        assert_eq!(scope.join_intent("invite".into(), "Mira".into()), None);
        current.feedback = JoinFeedback::Rejected("Invitation rejected; edit and retry");
        current.stamp.sequence += 1;
        scope.commit(&current);
        assert_eq!(
            scope.join_intent("corrected invitation".into(), "Mira".into()),
            Some(JoinPhaseIntent::Join {
                stamp: current.stamp,
                invitation: "corrected invitation".into(),
                player_name: "Mira".into(),
            })
        );
        // The caller's offline label does not erase an admitted roster or infer
        // membership/permissions from connectivity.
        current.stage = JoinStage::Lobby;
        current.join_enabled = true;
        scope.commit(&current);
        assert_eq!(scope.join_intent("invite".into(), "Mira".into()), None);
        assert_eq!(scope.lobby_intent(), None);
    }

    #[test]
    fn suspended_render_and_double_disposal_fence_intents() {
        let mut current = view(&[]);
        let mut scope = JoinScope::new(&current);
        current.stamp.sequence += 1;
        scope.begin_render(current.stamp);
        assert_eq!(scope.join_intent("invite".into(), "Mira".into()), None);
        scope.commit(&current);
        assert!(scope.join_intent("invite".into(), "Mira".into()).is_some());
        scope.dispose();
        scope.dispose();
        assert_eq!(scope.join_intent("invite".into(), "Mira".into()), None);
        assert_eq!(
            scope.check(JoinStamp {
                generation: 7,
                sequence: 2
            }),
            Err(JoinValidationError::Disposed)
        );
    }

    #[test]
    fn failed_owner_render_irreversibly_retires_old_generation_and_allows_new_retry() {
        let mut current = view(&[]);
        current.stage = JoinStage::Lobby;
        current.lobby_offer = Some(LobbyOffer {
            key: "old-ready",
            label: "Ready",
            enabled: true,
            pending: false,
        });
        let mut scope = JoinScope::new(&current);
        let replacement = JoinStamp {
            generation: 8,
            sequence: 1,
        };
        scope
            .begin_replacement(replacement)
            .expect("advanced owner");
        // Simulate a DOM error by deliberately omitting commit after retirement.
        assert_eq!(
            scope.check(JoinStamp {
                generation: 7,
                sequence: u64::MAX
            }),
            Ok(JoinUpdate::StaleGeneration)
        );
        assert_eq!(
            scope.join_intent("old invite".into(), "old name".into()),
            None
        );
        assert_eq!(scope.lobby_intent(), None);
        assert_eq!(scope.check(replacement), Ok(JoinUpdate::Applied));
        assert_eq!(
            scope.check(JoinStamp {
                generation: 8,
                sequence: 0
            }),
            Ok(JoinUpdate::StaleSequence)
        );
        let mut recovered = view(&[]);
        recovered.stamp = replacement;
        scope.commit(&recovered);
        assert_eq!(scope.check(replacement), Ok(JoinUpdate::StaleSequence));
        assert_eq!(
            scope.check(JoinStamp {
                generation: 7,
                sequence: u64::MAX
            }),
            Ok(JoinUpdate::StaleGeneration)
        );
        assert_eq!(
            scope.join_intent("new invite".into(), "new name".into()),
            Some(JoinPhaseIntent::Join {
                stamp: replacement,
                invitation: "new invite".into(),
                player_name: "new name".into(),
            })
        );
    }

    #[test]
    fn failed_same_owner_attempt_rejects_committed_replay_and_only_recovers_current_attempt() {
        let mut current = view(&[]);
        current.stage = JoinStage::Lobby;
        current.lobby_offer = Some(LobbyOffer {
            key: "old-ready",
            label: "Ready",
            enabled: true,
            pending: false,
        });
        let mut scope = JoinScope::new(&current);
        let failed = JoinStamp {
            generation: current.stamp.generation,
            sequence: 2,
        };
        scope.begin_render(failed);
        // Simulate fallible reconciliation without commit. The previous valid
        // view and its offer cannot recover a newer failed render.
        assert_eq!(scope.check(current.stamp), Ok(JoinUpdate::StaleSequence));
        assert_eq!(scope.check(failed), Ok(JoinUpdate::Applied));
        assert_eq!(scope.lobby_intent(), None);
        assert_eq!(scope.join_intent("invite".into(), "name".into()), None);
        let newer = JoinStamp {
            generation: failed.generation,
            sequence: 3,
        };
        scope.begin_render(newer);
        assert_eq!(scope.check(failed), Ok(JoinUpdate::StaleSequence));
        assert_eq!(scope.check(newer), Ok(JoinUpdate::Applied));
        current.stamp = newer;
        current.lobby_offer = Some(LobbyOffer {
            key: "current-ready",
            label: "Ready",
            enabled: true,
            pending: false,
        });
        scope.commit(&current);
        assert_eq!(scope.check(newer), Ok(JoinUpdate::StaleSequence));
        assert_eq!(
            scope.lobby_intent(),
            Some(JoinPhaseIntent::LobbyAction {
                stamp: newer,
                offer_key: "current-ready".into(),
            })
        );
    }

    #[test]
    fn roster_bounds_duplicate_keys_and_stage_are_validated_before_render() {
        let participants: Vec<_> = (0..8)
            .map(|_| LobbyParticipant {
                key: "same",
                name: "Aster",
                sigil: "A",
                readiness: "Ready",
                presence: "Sleeping",
            })
            .collect();
        assert_eq!(
            view(&participants).validate(limits()),
            Err(JoinValidationError::DuplicateParticipant)
        );
        assert_eq!(
            view(&participants).validate(CampaignLimits {
                max_members: 7,
                ..limits()
            }),
            Err(JoinValidationError::ResourceLimit)
        );
        let mut current = view(&[]);
        current.lobby_offer = Some(LobbyOffer {
            key: "offer",
            label: "Start",
            enabled: true,
            pending: false,
        });
        assert_eq!(
            current.validate(limits()),
            Err(JoinValidationError::InvalidStage)
        );
        current.lobby_offer = None;
        current.feedback = JoinFeedback::Rejected(" ");
        assert_eq!(
            current.validate(limits()),
            Err(JoinValidationError::EmptyText)
        );
    }

    #[test]
    fn draft_bound_uses_utf16_and_does_not_invent_invitation_or_name_rules() {
        assert_eq!(validate_join_drafts("", ""), Ok(()));
        assert_eq!(
            validate_join_drafts("opaque/server-owned?invitation", "<Mira & Aster>"),
            Ok(())
        );
        let at_limit = "😀".repeat(MAX_DRAFT_UTF16_UNITS / 2);
        assert_eq!(validate_join_drafts(&at_limit, "Mira"), Ok(()));
        assert_eq!(
            validate_join_drafts(&format!("{at_limit}x"), "Mira"),
            Err(JoinValidationError::DraftTooLong)
        );
    }
}
