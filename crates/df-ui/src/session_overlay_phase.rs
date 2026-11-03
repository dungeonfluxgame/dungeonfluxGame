use std::{collections::BTreeSet, fmt};

use crate::FeedbackView;

/// Connectivity is supplied by the connection owner, independently of any
/// operation outcome. Reconnecting never implies a confirmed or retriable action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionConnection {
    Connecting,
    Connected,
    Reconnecting,
    Offline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionBookendKind {
    Recap,
    SpeculativeTrailer,
    CriticalCue,
}

/// Plain, already localized and audience-safe content selected by the owner.
/// Position is a supplied caption, never a client clock or completion claim.
pub struct SessionBookend<'a> {
    pub key: &'a str,
    pub kind: SessionBookendKind,
    pub title: &'a str,
    pub context: &'a str,
    pub caption: &'a str,
    pub attribution: &'a str,
    pub position: &'a str,
    pub still: Option<(&'a str, &'a str)>,
    pub skip_offer: Option<SessionOverlayOffer<'a>>,
}

/// An opaque current owner-advertised selection. It carries no host permission,
/// receipt, retry authority or command semantics. The caller revalidates on use.
#[derive(Clone, Copy, Debug)]
pub struct SessionOverlayOffer<'a> {
    pub key: &'a str,
    pub label: &'a str,
    pub enabled: bool,
    pub pending: bool,
}

/// Bounded presentation props for a current accepted view. Generation identifies
/// the mount owner, not a game epoch. A changed owner requires disposal/remount.
pub struct SessionOverlayView<'a> {
    pub generation: u64,
    pub revision: u64,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub connection: SessionConnection,
    pub connection_label: &'a str,
    pub connection_detail: &'a str,
    pub operation: Option<FeedbackView<'a>>,
    pub bookend: Option<SessionBookend<'a>>,
    pub host_context: &'a str,
    pub host_offers: &'a [SessionOverlayOffer<'a>],
    pub reduced_motion: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionOverlayValidationError {
    EmptyText,
    TextLimit,
    OfferLimit,
    DuplicateOffer,
    InvalidStill,
    WrongGeneration,
    StaleView,
    Disposed,
}

impl fmt::Display for SessionOverlayValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyText => "session overlay text is empty",
            Self::TextLimit => "session overlay text exceeds its bound",
            Self::OfferLimit => "session overlay has too many host selections",
            Self::DuplicateOffer => "session overlay selection keys repeat",
            Self::InvalidStill => "session overlay still must use a bounded local asset path",
            Self::WrongGeneration => "session overlay owner was replaced",
            Self::StaleView => "session overlay view is stale",
            Self::Disposed => "session overlay was disposed",
        })
    }
}

impl std::error::Error for SessionOverlayValidationError {}

pub const MAX_SESSION_HOST_OFFERS: usize = 4;
const MAX_TEXT_BYTES: usize = 4096;
const MAX_KEY_BYTES: usize = 128;

impl SessionOverlayView<'_> {
    pub fn validate(&self) -> Result<(), SessionOverlayValidationError> {
        for value in [
            self.title,
            self.subtitle,
            self.connection_label,
            self.connection_detail,
            self.host_context,
        ] {
            validate_text(value, MAX_TEXT_BYTES)?;
        }
        if self.host_offers.len() > MAX_SESSION_HOST_OFFERS {
            return Err(SessionOverlayValidationError::OfferLimit);
        }
        let mut keys = BTreeSet::new();
        for offer in self.host_offers {
            validate_offer(offer)?;
            if !keys.insert(offer.key) {
                return Err(SessionOverlayValidationError::DuplicateOffer);
            }
        }
        if let Some(feedback) = self.operation {
            match feedback {
                FeedbackView::Pending(value)
                | FeedbackView::Uncertain(value)
                | FeedbackView::Refused(value) => validate_text(value, MAX_TEXT_BYTES)?,
                FeedbackView::Error {
                    uncertainty,
                    message,
                } => {
                    validate_text(uncertainty, MAX_TEXT_BYTES)?;
                    validate_text(message, MAX_TEXT_BYTES)?;
                }
            }
        }
        if let Some(bookend) = &self.bookend {
            validate_text(bookend.key, MAX_KEY_BYTES)?;
            for value in [
                bookend.title,
                bookend.context,
                bookend.caption,
                bookend.attribution,
                bookend.position,
            ] {
                validate_text(value, MAX_TEXT_BYTES)?;
            }
            if let Some((path, alt)) = bookend.still {
                if path.len() > 2048
                    || !path.starts_with("assets/")
                    || path.contains("..")
                    || !path
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"/-_.".contains(&byte))
                {
                    return Err(SessionOverlayValidationError::InvalidStill);
                }
                validate_text(alt, MAX_TEXT_BYTES)?;
            }
            if let Some(offer) = &bookend.skip_offer {
                validate_offer(offer)?;
                if !keys.insert(offer.key) {
                    return Err(SessionOverlayValidationError::DuplicateOffer);
                }
            }
        }
        Ok(())
    }
}

fn validate_offer(offer: &SessionOverlayOffer<'_>) -> Result<(), SessionOverlayValidationError> {
    validate_text(offer.key, MAX_KEY_BYTES)?;
    validate_text(offer.label, MAX_TEXT_BYTES)
}

fn validate_text(value: &str, bound: usize) -> Result<(), SessionOverlayValidationError> {
    if value.trim().is_empty() {
        return Err(SessionOverlayValidationError::EmptyText);
    }
    if value.len() > bound {
        return Err(SessionOverlayValidationError::TextLimit);
    }
    Ok(())
}

/// Callback captures only the currently rendered presentation selection. No
/// transport action is dispatched by this component.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionOverlaySelection {
    pub generation: u64,
    pub revision: u64,
    pub key: String,
}

#[cfg(any(target_arch = "wasm32", test))]
struct SessionOverlayOwner {
    generation: u64,
    revision: u64,
    disposed: bool,
    selections: Vec<Option<SessionOverlaySelection>>,
    host_keys: Vec<Option<String>>,
}

#[cfg(any(target_arch = "wasm32", test))]
impl SessionOverlayOwner {
    fn new(view: &SessionOverlayView<'_>) -> Self {
        Self {
            generation: view.generation,
            revision: view.revision,
            disposed: false,
            selections: vec![None; MAX_SESSION_HOST_OFFERS + 1],
            host_keys: vec![None; MAX_SESSION_HOST_OFFERS],
        }
    }

    fn require(&self, view: &SessionOverlayView<'_>) -> Result<(), SessionOverlayValidationError> {
        if self.disposed {
            return Err(SessionOverlayValidationError::Disposed);
        }
        if view.generation != self.generation {
            return Err(SessionOverlayValidationError::WrongGeneration);
        }
        if view.revision < self.revision {
            return Err(SessionOverlayValidationError::StaleView);
        }
        view.validate()
    }

    fn host_slots<'a>(
        &self,
        view: &'a SessionOverlayView<'a>,
    ) -> Vec<Option<&'a SessionOverlayOffer<'a>>> {
        let mut assigned = BTreeSet::new();
        let mut slots: Vec<_> = self
            .host_keys
            .iter()
            .map(|key| {
                let offer = key
                    .as_ref()
                    .and_then(|key| view.host_offers.iter().find(|offer| offer.key == key));
                if let Some(offer) = offer {
                    assigned.insert(offer.key);
                }
                offer
            })
            .collect();
        for offer in view.host_offers {
            if !assigned.contains(offer.key)
                && let Some(slot) = slots.iter_mut().find(|slot| slot.is_none())
            {
                *slot = Some(offer);
                assigned.insert(offer.key);
            }
        }
        slots
    }

    fn reconcile(&mut self, view: &SessionOverlayView<'_>) {
        let slots = self.host_slots(view);
        self.host_keys = slots
            .iter()
            .map(|offer| offer.map(|offer| offer.key.to_owned()))
            .collect();
        self.revision = view.revision;
        self.selections.fill(None);
        // A reconnect view cannot revive selections from a prior connection.
        // The current connection owner must supply fresh offers after resync.
        if view.connection != SessionConnection::Connected {
            return;
        }
        for (slot, offer) in self.selections.iter_mut().zip(slots) {
            if let Some(offer) = offer
                && offer.enabled
                && !offer.pending
            {
                *slot = Some(SessionOverlaySelection {
                    generation: self.generation,
                    revision: self.revision,
                    key: offer.key.to_owned(),
                });
            }
        }
        if let Some(offer) = view
            .bookend
            .as_ref()
            .and_then(|bookend| bookend.skip_offer.as_ref())
            && offer.enabled
            && !offer.pending
            && let Some(slot) = self.selections.last_mut()
        {
            *slot = Some(SessionOverlaySelection {
                generation: self.generation,
                revision: self.revision,
                key: offer.key.to_owned(),
            });
        }
    }

    fn selection(&self, index: usize) -> Option<SessionOverlaySelection> {
        if self.disposed {
            return None;
        }
        self.selections.get(index).cloned().flatten()
    }

    fn dispose(&mut self) {
        self.disposed = true;
        self.selections.fill(None);
        self.host_keys.fill(None);
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::{
        ActionView, ControlError, ControlledAction, FeedbackScope, OperationFeedback, UiError,
    };
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{Document, Element, Event, HtmlElement, KeyboardEvent, Node};

    #[derive(Debug)]
    pub enum SessionOverlayError {
        Validation(SessionOverlayValidationError),
        Control(ControlError),
        Dom(UiError),
    }
    impl fmt::Display for SessionOverlayError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Validation(error) => fmt::Display::fmt(error, formatter),
                Self::Control(error) => fmt::Display::fmt(error, formatter),
                Self::Dom(error) => fmt::Display::fmt(error, formatter),
            }
        }
    }
    impl std::error::Error for SessionOverlayError {}
    impl From<SessionOverlayValidationError> for SessionOverlayError {
        fn from(error: SessionOverlayValidationError) -> Self {
            Self::Validation(error)
        }
    }
    impl From<ControlError> for SessionOverlayError {
        fn from(error: ControlError) -> Self {
            Self::Control(error)
        }
    }
    impl From<UiError> for SessionOverlayError {
        fn from(error: UiError) -> Self {
            Self::Dom(error)
        }
    }
    impl From<wasm_bindgen::JsValue> for SessionOverlayError {
        fn from(error: wasm_bindgen::JsValue) -> Self {
            Self::Dom(UiError::Browser(error))
        }
    }

    type SelectionCallback = Box<dyn FnMut(SessionOverlaySelection)>;

    /// Persistent nonmodal overlay. Essential input has its own stable visible slot;
    /// reconnect, cinematic captions and host navigation never trap or disable it.
    /// Call dispose explicitly to observe removal failures. Drop fences callbacks.
    pub struct SessionOverlayPhase {
        root: Element,
        title: Element,
        subtitle: Element,
        connection: Element,
        connection_detail: Element,
        feedback: OperationFeedback,
        feedback_scope: FeedbackScope,
        drawer: Element,
        bookend: Element,
        kind: Element,
        bookend_title: Element,
        context: Element,
        image: Element,
        caption: Element,
        attribution: Element,
        position: Element,
        host: Element,
        host_context: Element,
        essential: Element,
        open: Rc<ControlledAction>,
        close: Rc<ControlledAction>,
        actions: Vec<Rc<ControlledAction>>,
        owner: Rc<RefCell<SessionOverlayOwner>>,
        callback: Rc<RefCell<Option<SelectionCallback>>>,
        callback_registered: Cell<bool>,
        focus_return: Rc<RefCell<Option<HtmlElement>>>,
        key_listener: Option<Closure<dyn FnMut(Event)>>,
    }

    impl SessionOverlayPhase {
        pub fn create(
            document: &Document,
            view: &SessionOverlayView<'_>,
        ) -> Result<Self, SessionOverlayError> {
            view.validate()?;
            let root = document.create_element("section")?;
            root.set_class_name("df-session-phase");
            let style = child(document, &root, "style", "", None)?;
            style.set_text_content(Some(crate::session_overlay_phase_theme::STYLESHEET));
            let header = child(document, &root, "header", "session-header", None)?;
            child(document, &header, "p", "session-brand", Some("DungeonFlux"))?;
            let title = child(document, &header, "h1", "", None)?;
            let subtitle = child(document, &header, "p", "session-subtitle", None)?;
            let open = Rc::new(ControlledAction::create(
                document,
                ActionView {
                    label: "Session details",
                    enabled: true,
                    pending: false,
                },
            )?);
            header.append_child(open.element())?;
            let recovery = child(document, &root, "section", "session-recovery", None)?;
            recovery.set_attribute("aria-label", "Connection and action status")?;
            let connection = child(document, &recovery, "p", "connection-label", None)?;
            connection.set_attribute("role", "status")?;
            let connection_detail = child(document, &recovery, "p", "connection-detail", None)?;
            let (feedback, feedback_scope) = OperationFeedback::create(
                document,
                FeedbackView::Uncertain("No operation status supplied"),
            )?;
            recovery.append_child(feedback.root())?;
            let essential = child(document, &root, "section", "session-essential", None)?;
            essential.set_attribute("aria-label", "Current session input")?;
            let drawer = child(document, &root, "section", "session-drawer", None)?;
            drawer.set_attribute("role", "dialog")?;
            drawer.set_attribute("aria-modal", "false")?;
            drawer.set_attribute("aria-label", "Session details")?;
            drawer.set_attribute("hidden", "")?;
            let close = Rc::new(ControlledAction::create(
                document,
                ActionView {
                    label: "Close details",
                    enabled: true,
                    pending: false,
                },
            )?);
            drawer.append_child(close.element())?;
            let bookend = child(document, &drawer, "figure", "session-bookend", None)?;
            let image = child(document, &bookend, "img", "session-still", None)?;
            let copy = child(document, &bookend, "figcaption", "bookend-copy", None)?;
            let kind = child(document, &copy, "p", "bookend-kind", None)?;
            let bookend_title = child(document, &copy, "h2", "", None)?;
            let context = child(document, &copy, "p", "bookend-context", None)?;
            let caption = child(document, &copy, "blockquote", "bookend-caption", None)?;
            let attribution = child(document, &copy, "p", "bookend-attribution", None)?;
            let position = child(document, &copy, "p", "bookend-position", None)?;
            let host = child(document, &drawer, "section", "session-host", None)?;
            child(document, &host, "h2", "", Some("Host controls"))?;
            let host_context = child(document, &host, "p", "", None)?;
            let owner = Rc::new(RefCell::new(SessionOverlayOwner::new(view)));
            let callback = Rc::new(RefCell::new(None::<SelectionCallback>));
            let mut actions = Vec::new();
            for index in 0..=MAX_SESSION_HOST_OFFERS {
                let action = Rc::new(ControlledAction::create(
                    document,
                    ActionView {
                        label: "Unavailable",
                        enabled: false,
                        pending: false,
                    },
                )?);
                let selection_owner = Rc::clone(&owner);
                let current_callback = Rc::clone(&callback);
                action.on_activate(move || {
                    let selection = selection_owner.borrow().selection(index);
                    if let Some(selection) = selection {
                        let saved = current_callback.borrow_mut().take();
                        if let Some(mut saved) = saved {
                            saved(selection);
                            if !selection_owner.borrow().disposed {
                                *current_callback.borrow_mut() = Some(saved);
                            }
                        }
                    }
                })?;
                if index == MAX_SESSION_HOST_OFFERS {
                    copy.append_child(action.element())?;
                } else {
                    host.append_child(action.element())?;
                }
                actions.push(action);
            }
            let local_error = child(document, &root, "p", "session-local-error", None)?;
            local_error.set_attribute("role", "alert")?;
            let focus_return = Rc::new(RefCell::new(None::<HtmlElement>));
            let opened = drawer.clone();
            let opened_document = document.clone();
            let restored = Rc::clone(&focus_return);
            let open_focus = close.element().clone();
            let open_error = local_error.clone();
            open.on_activate(move || {
                *restored.borrow_mut() = opened_document
                    .active_element()
                    .and_then(|element| element.dyn_into::<HtmlElement>().ok());
                let result = opened
                    .remove_attribute("hidden")
                    .and_then(|()| open_focus.focus());
                if result.is_err() {
                    open_error.set_text_content(Some("Session details could not receive focus."));
                }
            })?;
            let closed = drawer.clone();
            let restored = Rc::clone(&focus_return);
            let close_error = local_error.clone();
            close.on_activate(move || {
                if close_drawer(&closed, &restored).is_err() {
                    close_error.set_text_content(Some("Session details could not restore focus."));
                }
            })?;
            let key_drawer = drawer.clone();
            let key_focus = Rc::clone(&focus_return);
            let key_owner = Rc::clone(&owner);
            let key_error = local_error.clone();
            let key_listener = Closure::wrap(Box::new(move |event: Event| {
                if key_owner.borrow().disposed {
                    return;
                }
                if let Some(event) = event.dyn_ref::<KeyboardEvent>()
                    && event.key() == "Escape"
                    && !key_drawer.has_attribute("hidden")
                {
                    event.prevent_default();
                    if close_drawer(&key_drawer, &key_focus).is_err() {
                        key_error
                            .set_text_content(Some("Session details could not restore focus."));
                    }
                }
            }) as Box<dyn FnMut(Event)>);
            drawer.add_event_listener_with_callback(
                "keydown",
                key_listener.as_ref().unchecked_ref(),
            )?;
            let phase = Self {
                root,
                title,
                subtitle,
                connection,
                connection_detail,
                feedback,
                feedback_scope,
                drawer,
                bookend,
                kind,
                bookend_title,
                context,
                image,
                caption,
                attribution,
                position,
                host,
                host_context,
                essential,
                open,
                close,
                actions,
                owner,
                callback,
                callback_registered: Cell::new(false),
                focus_return,
                key_listener: Some(key_listener),
            };
            phase.update(view)?;
            Ok(phase)
        }

        pub fn root(&self) -> &Element {
            &self.root
        }
        pub fn essential_input(&self) -> &Element {
            &self.essential
        }

        pub fn on_selection(
            &self,
            callback: impl FnMut(SessionOverlaySelection) + 'static,
        ) -> Result<(), SessionOverlayError> {
            if self.owner.borrow().disposed {
                return Err(SessionOverlayValidationError::Disposed.into());
            }
            if self.callback_registered.get() {
                return Err(ControlError::CallbackAlreadyRegistered.into());
            }
            *self.callback.borrow_mut() = Some(Box::new(callback));
            self.callback_registered.set(true);
            Ok(())
        }

        /// Validate before touching nodes; temporarily fence selections while
        /// updating. Same-phase and reconnect updates preserve input and focus.
        pub fn update(&self, view: &SessionOverlayView<'_>) -> Result<(), SessionOverlayError> {
            self.owner.borrow().require(view)?;
            let host_slots = self.owner.borrow().host_slots(view);
            let focused_action = self
                .root
                .owner_document()
                .and_then(|document| document.active_element())
                .and_then(|active| {
                    if !self.root.contains(Some(&active)) {
                        return None;
                    }
                    self.actions
                        .iter()
                        .enumerate()
                        .find(|(_, action)| action.element().is_same_node(Some(&active)))
                        .map(|(index, action)| {
                            (index, action.element().get_attribute("data-offer-key"))
                        })
                });
            // Fence delivery through the presentation owner, not a temporary
            // disabled DOM state that can blur a surviving keyboard target.
            self.owner.borrow_mut().selections.fill(None);
            replace_text(&self.title, Some(view.title));
            replace_text(&self.subtitle, Some(view.subtitle));
            replace_text(&self.connection, Some(view.connection_label));
            replace_text(&self.connection_detail, Some(view.connection_detail));
            self.root.set_attribute(
                "data-session-connection",
                match view.connection {
                    SessionConnection::Connecting => "connecting",
                    SessionConnection::Connected => "connected",
                    SessionConnection::Reconnecting => "reconnecting",
                    SessionConnection::Offline => "offline",
                },
            )?;
            self.root.set_attribute(
                "data-reduced-motion",
                if view.reduced_motion { "true" } else { "false" },
            )?;
            scrub_text_nodes(self.feedback.root().as_ref());
            if let Some(feedback) = view.operation {
                self.feedback.update(&self.feedback_scope, feedback)?;
                self.feedback.root().remove_attribute("hidden")?;
            } else {
                self.feedback.update(
                    &self.feedback_scope,
                    FeedbackView::Uncertain("No operation status supplied"),
                )?;
                self.feedback.root().set_attribute("hidden", "")?;
            }
            replace_text(&self.host_context, Some(view.host_context));
            self.host
                .set_attribute("data-offer-count", &view.host_offers.len().to_string())?;
            for (index, action) in self.actions.iter().enumerate() {
                let offer = if index == MAX_SESSION_HOST_OFFERS {
                    view.bookend
                        .as_ref()
                        .and_then(|bookend| bookend.skip_offer.as_ref())
                } else {
                    host_slots.get(index).copied().flatten()
                };
                if let Some(offer) = offer {
                    scrub_text_nodes(action.element().as_ref());
                    action.update(ActionView {
                        label: offer.label,
                        enabled: offer.enabled && view.connection == SessionConnection::Connected,
                        pending: offer.pending,
                    })?;
                    action
                        .element()
                        .set_attribute("data-offer-key", offer.key)?;
                    action.element().remove_attribute("hidden")?;
                } else {
                    scrub_text_nodes(action.element().as_ref());
                    action.update(ActionView {
                        label: "Unavailable",
                        enabled: false,
                        pending: false,
                    })?;
                    replace_text(action.element().as_ref(), None);
                    action.element().remove_attribute("data-offer-key")?;
                    action.element().set_attribute("hidden", "")?;
                }
            }
            if let Some(bookend) = &view.bookend {
                self.bookend.remove_attribute("hidden")?;
                self.bookend.set_attribute("data-cue-key", bookend.key)?;
                replace_text(
                    &self.kind,
                    Some(match bookend.kind {
                        SessionBookendKind::Recap => "Previously · Authorized recap",
                        SessionBookendKind::SpeculativeTrailer => {
                            "A possible future · Speculative trailer"
                        }
                        SessionBookendKind::CriticalCue => {
                            "A moment in the story · Committed event"
                        }
                    }),
                );
                replace_text(&self.bookend_title, Some(bookend.title));
                replace_text(&self.context, Some(bookend.context));
                replace_text(&self.caption, Some(bookend.caption));
                replace_text(&self.attribution, Some(bookend.attribution));
                replace_text(&self.position, Some(bookend.position));
                if let Some((path, alt)) = bookend.still {
                    // No decode callback publishes content later; the current view
                    // assigns the one image node and captions remain its fallback.
                    self.image.set_attribute("src", path)?;
                    self.image.set_attribute("alt", alt)?;
                    self.image.remove_attribute("hidden")?;
                } else {
                    self.image.remove_attribute("src")?;
                    self.image.remove_attribute("alt")?;
                    self.image.set_attribute("hidden", "")?;
                }
            } else {
                self.bookend.set_attribute("hidden", "")?;
                self.bookend.remove_attribute("data-cue-key")?;
                self.image.remove_attribute("src")?;
                self.image.remove_attribute("alt")?;
                self.image.set_attribute("hidden", "")?;
                for element in [
                    &self.kind,
                    &self.bookend_title,
                    &self.context,
                    &self.caption,
                    &self.attribution,
                    &self.position,
                ] {
                    replace_text(element, None);
                }
            }
            self.root
                .set_attribute("data-view-revision", &view.revision.to_string())?;
            self.owner.borrow_mut().reconcile(view);
            if let Some((index, key)) = focused_action {
                let surviving_action = self.actions.get(index).filter(|action| {
                    key.is_some()
                        && self.root.contains(Some(action.element()))
                        && !action.element().disabled()
                        && !action.element().has_attribute("hidden")
                        && action.element().get_attribute("data-offer-key") == key
                        && !self.drawer.has_attribute("hidden")
                        && (index != MAX_SESSION_HOST_OFFERS
                            || !self.bookend.has_attribute("hidden"))
                });
                if let Some(action) = surviving_action {
                    let retained = self
                        .root
                        .owner_document()
                        .and_then(|document| document.active_element())
                        .is_some_and(|active| active.is_same_node(Some(action.element())));
                    if !retained {
                        action.element().focus()?;
                    }
                } else {
                    // An obsolete/disabled selection cannot keep keyboard focus;
                    // choose local presentation navigation, never another offer.
                    if self.drawer.has_attribute("hidden") {
                        self.open.element().focus()?;
                    } else {
                        self.close.element().focus()?;
                    }
                }
            }
            Ok(())
        }

        pub fn dispose(&mut self) -> Result<(), SessionOverlayError> {
            // Fence first, then scrub before any removal: callers can retain DOM
            // descendants and even detached Text nodes across owner replacement.
            self.owner.borrow_mut().dispose();
            self.callback.borrow_mut().take();
            self.focus_return.borrow_mut().take();
            let mut failure = self.scrub_owned().err();
            for action in std::iter::once(&self.open)
                .chain(std::iter::once(&self.close))
                .chain(self.actions.iter())
            {
                if let Err(error) = action.dispose() {
                    failure.get_or_insert(error.into());
                }
            }
            if let Err(error) = self.feedback.dispose() {
                failure.get_or_insert(error.into());
            }
            if let Some(listener) = self.key_listener.as_ref() {
                match self.drawer.remove_event_listener_with_callback(
                    "keydown",
                    listener.as_ref().unchecked_ref(),
                ) {
                    Ok(()) => self.key_listener = None,
                    Err(error) => {
                        failure.get_or_insert(error.into());
                    }
                }
            }
            self.root.set_text_content(None);
            self.root.remove();
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        fn scrub_owned(&self) -> Result<(), SessionOverlayError> {
            scrub_text_nodes_except(self.root.as_ref(), Some(self.essential.as_ref()));
            for element in [
                &self.title,
                &self.subtitle,
                &self.connection,
                &self.connection_detail,
                &self.kind,
                &self.bookend_title,
                &self.context,
                &self.caption,
                &self.attribution,
                &self.position,
                &self.host_context,
                self.feedback.root(),
            ] {
                replace_text(element, None);
            }
            let mut failure = None;
            for (element, attributes) in [
                (
                    &self.root,
                    &[
                        "data-view-revision",
                        "data-session-connection",
                        "data-reduced-motion",
                    ][..],
                ),
                (&self.bookend, &["data-cue-key"][..]),
                (&self.image, &["src", "alt", "hidden"][..]),
                (&self.host, &["data-offer-count"][..]),
            ] {
                for attribute in attributes {
                    if let Err(error) = element.remove_attribute(attribute) {
                        failure.get_or_insert(SessionOverlayError::from(error));
                    }
                }
            }
            for action in std::iter::once(&self.open)
                .chain(std::iter::once(&self.close))
                .chain(self.actions.iter())
            {
                replace_text(action.element().as_ref(), None);
                for attribute in ["data-offer-key", "aria-busy", "hidden"] {
                    if let Err(error) = action.element().remove_attribute(attribute) {
                        failure.get_or_insert(SessionOverlayError::from(error));
                    }
                }
            }
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }

    impl Drop for SessionOverlayPhase {
        fn drop(&mut self) {
            if self.dispose().is_err()
                && let Some(listener) = self.key_listener.take()
            {
                listener.forget();
            }
        }
    }

    // set_text_content removes prior Text nodes; it does not erase references
    // retained elsewhere. Scrub them in place before replacement or detachment.
    // These targets are this component's fixed prose/control/feedback subtrees,
    // never the caller-owned essential-input slot.
    fn scrub_text_nodes(node: &Node) {
        scrub_text_nodes_except(node, None);
    }

    fn scrub_text_nodes_except(node: &Node, excluded: Option<&Node>) {
        if excluded.is_some_and(|excluded| node.is_same_node(Some(excluded))) {
            return;
        }
        if node.node_type() == Node::TEXT_NODE {
            node.set_node_value(Some(""));
        }
        let mut child = node.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            scrub_text_nodes_except(&current, excluded);
        }
    }

    fn replace_text(element: &Element, value: Option<&str>) {
        scrub_text_nodes(element.as_ref());
        element.set_text_content(value);
    }

    fn close_drawer(
        drawer: &Element,
        focus: &RefCell<Option<HtmlElement>>,
    ) -> Result<(), wasm_bindgen::JsValue> {
        drawer.set_attribute("hidden", "")?;
        if let Some(element) = focus.borrow_mut().take()
            && element.is_connected()
        {
            element.focus()?;
        }
        Ok(())
    }

    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
        text: Option<&str>,
    ) -> Result<Element, wasm_bindgen::JsValue> {
        let element = document.create_element(tag)?;
        element.set_class_name(class);
        element.set_text_content(text);
        parent.append_child(&element)?;
        Ok(element)
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{SessionOverlayError, SessionOverlayPhase};

#[cfg(test)]
mod tests {
    use super::*;
    fn view<'a>(offers: &'a [SessionOverlayOffer<'a>]) -> SessionOverlayView<'a> {
        SessionOverlayView {
            generation: 8,
            revision: 4,
            title: "The Drowned Lantern",
            subtitle: "Session interlude",
            connection: SessionConnection::Connected,
            connection_label: "Connected",
            connection_detail: "Current permitted view",
            operation: Some(FeedbackView::Error {
                uncertainty: "Outcome remains unknown",
                message: "Receipt lookup unavailable",
            }),
            bookend: None,
            host_context: "Current allowed host actions only",
            host_offers: offers,
            reduced_motion: false,
        }
    }
    #[test]
    fn connection_recovery_cannot_restore_old_offer_or_resolve_unknown() {
        let offers = [SessionOverlayOffer {
            key: "pause-current",
            label: "Request pause",
            enabled: true,
            pending: false,
        }];
        let mut current = view(&offers);
        let mut owner = SessionOverlayOwner::new(&current);
        owner.reconcile(&current);
        assert_eq!(owner.selection(0).expect("current selection").revision, 4);
        current.connection = SessionConnection::Reconnecting;
        current.revision = 5;
        owner.require(&current).expect("valid recovery view");
        owner.reconcile(&current);
        assert!(owner.selection(0).is_none());
        assert!(matches!(
            current.operation,
            Some(FeedbackView::Error {
                uncertainty: "Outcome remains unknown",
                message: "Receipt lookup unavailable"
            })
        ));
        current.connection = SessionConnection::Connected;
        current.host_offers = &[];
        current.revision = 6;
        owner.reconcile(&current);
        assert!(owner.selection(0).is_none());
    }
    #[test]
    fn pending_revoked_stale_and_disposed_selections_never_dispatch() {
        let offers = [SessionOverlayOffer {
            key: "pause",
            label: "Pause",
            enabled: true,
            pending: true,
        }];
        let mut current = view(&offers);
        let mut owner = SessionOverlayOwner::new(&current);
        owner.reconcile(&current);
        assert!(owner.selection(0).is_none());
        current.revision = 3;
        assert_eq!(
            owner.require(&current),
            Err(SessionOverlayValidationError::StaleView)
        );
        current.revision = 4;
        current.generation = 9;
        assert_eq!(
            owner.require(&current),
            Err(SessionOverlayValidationError::WrongGeneration)
        );
        owner.dispose();
        owner.dispose();
        assert_eq!(
            owner.require(&current),
            Err(SessionOverlayValidationError::Disposed)
        );
        assert!(owner.selection(0).is_none());
    }
    #[test]
    fn offer_reorder_retains_the_key_at_its_mounted_slot() {
        let first = [
            SessionOverlayOffer {
                key: "pause",
                label: "Pause",
                enabled: true,
                pending: false,
            },
            SessionOverlayOffer {
                key: "end",
                label: "End",
                enabled: true,
                pending: false,
            },
        ];
        let reversed = [first[1], first[0]];
        let mut current = view(&first);
        let mut owner = SessionOverlayOwner::new(&current);
        owner.reconcile(&current);
        current.host_offers = &reversed;
        current.revision += 1;
        owner.reconcile(&current);
        assert_eq!(owner.selection(0).expect("same keyed node").key, "pause");
        assert_eq!(owner.selection(1).expect("same keyed node").key, "end");
    }
    #[test]
    fn bounds_duplicate_selection_and_script_still_are_rejected() {
        let repeated = [SessionOverlayOffer {
            key: "same",
            label: "Pause",
            enabled: true,
            pending: false,
        }; 2];
        assert_eq!(
            view(&repeated).validate(),
            Err(SessionOverlayValidationError::DuplicateOffer)
        );
        let excess = [SessionOverlayOffer {
            key: "same",
            label: "Pause",
            enabled: true,
            pending: false,
        }; 5];
        assert_eq!(
            view(&excess).validate(),
            Err(SessionOverlayValidationError::OfferLimit)
        );
        let mut current = view(&[]);
        current.bookend = Some(SessionBookend {
            key: "cue",
            kind: SessionBookendKind::Recap,
            title: "Recap",
            context: "Permitted events",
            caption: "A supplied caption",
            attribution: "Narrator",
            position: "Still",
            still: Some(("javascript:alert(1)", "Untrusted")),
            skip_offer: None,
        });
        assert_eq!(
            current.validate(),
            Err(SessionOverlayValidationError::InvalidStill)
        );
        current.bookend.as_mut().expect("bookend").still =
            Some(("assets/placeholder/scene-campfire.webp", "Campfire"));
        assert_eq!(current.validate(), Ok(()));
        current.subtitle = " ";
        assert_eq!(
            current.validate(),
            Err(SessionOverlayValidationError::EmptyText)
        );
    }
}
