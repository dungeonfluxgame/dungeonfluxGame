//! Bounded, borrowed combat presentation. These props are not a wire view, a rules
//! model, or a permission filter. The authorized player/display owner maps its
//! current view here; exact resource strings and initiative order are retained.

use std::{collections::BTreeSet, fmt};

use df_client::revisions::{ViewAcceptance, ViewStore};
use df_types::{ClientBindingId, RevisionLabel, SessionRevision};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CombatArt {
    Harbor,
    Crypt,
    FloodedTemple,
}
impl CombatArt {
    pub const fn asset_path(self) -> &'static str {
        match self {
            Self::Harbor => "assets/concept-art/combat-harbor-docks-wide-lanterns.webp",
            Self::Crypt => "assets/concept-art/scene-crypt-lich-king-confrontation.webp",
            Self::FloodedTemple => {
                "assets/concept-art/battlemap-flooded-hall-waterfall-temple.webp"
            }
        }
    }
    pub const fn description(self) -> &'static str {
        match self {
            Self::Harbor => "Lanterns illuminate a moonlit battle on the harbor docks",
            Self::Crypt => "A party confronts the lich king in an ancient crypt",
            Self::FloodedTemple => "A flooded temple with a waterfall and torchlit stone platforms",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CombatLimits {
    pub max_actors: usize,
    pub max_offers: usize,
    pub max_rolls: usize,
    pub max_resources_per_actor: usize,
    pub max_text_bytes: usize,
    pub max_total_text_bytes: usize,
}

pub struct CombatResource<'a> {
    pub label: &'a str,
    pub value: &'a str,
}
/// Supplied hit points. The bar only shows the ratio; no rule is evaluated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CombatHitPoints {
    pub current: u32,
    pub maximum: u32,
}
pub struct CombatActor<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub initiative: &'a str,
    pub status: &'a str,
    pub resources: &'a [CombatResource<'a>],
    /// Optional hit-point bar on the combatant card.
    pub hit_points: Option<CombatHitPoints>,
    /// Optional short ASCII token (letters, digits, `-`) published as `data-portrait`
    /// so an owner stylesheet can attach portrait art. The initiative badge remains
    /// the readable fallback when no art loads.
    pub portrait: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CombatOfferKind {
    Action,
    Roll,
    Reaction,
    /// A local on/off option that accompanies an action. Activation reports the same
    /// intent; the owner flips its own draft and publishes the new `selected` value.
    Choice {
        selected: bool,
    },
}

/// Exact supplied selection, forwarded with its accepted binding/revision. The
/// caller maps this to its advertised generated command; the UI cannot resolve it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CombatIntent {
    pub binding: ClientBindingId,
    pub revision: SessionRevision,
    pub kind: CombatOfferKind,
    pub offer: RevisionLabel,
    pub option: Option<RevisionLabel>,
    pub draft: Option<String>,
}

pub struct CombatOffer<'a> {
    /// Stable control key. Multiple choices for an offer have distinct control keys.
    pub key: &'a str,
    pub offer: &'a RevisionLabel,
    pub option: Option<&'a RevisionLabel>,
    pub kind: CombatOfferKind,
    pub label: &'a str,
    pub explanation: &'a str,
    pub enabled: bool,
    pub pending: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CombatTone {
    Success,
    Failure,
    Neutral,
}
/// Server-supplied outcome word for a roll, such as a hit or a miss.
#[derive(Clone, Copy)]
pub struct CombatVerdict<'a> {
    pub label: &'a str,
    pub tone: CombatTone,
}
pub struct CombatRoll<'a> {
    pub key: &'a str,
    pub label: &'a str,
    pub value: &'a str,
    pub explanation: &'a str,
    pub source: &'a str,
    /// Optional outcome ribbon. The first supplied roll is styled as the latest roll.
    pub verdict: Option<CombatVerdict<'a>>,
    /// The server's kept d20 face, so the value badge can mark a natural 20 or 1.
    pub die_face: Option<u8>,
    /// The resting-frame art behind the small die's number; `None` reads as before.
    pub die_rest: Option<&'a str>,
}
#[derive(Clone, Copy)]
pub struct CombatReaction<'a> {
    pub heading: &'a str,
    pub timing: &'a str,
    pub explanation: &'a str,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CombatFeedback<'a> {
    None,
    Pending(&'a str),
    Refused(&'a str),
}
#[derive(Clone, Copy)]
pub struct CombatDraft<'a> {
    pub key: &'a str,
    pub label: &'a str,
    pub enabled: bool,
    pub feedback: CombatFeedback<'a>,
}

/// Audience-safe, localized plain text. Active actor, timing, resources, offer
/// eligibility and committed rolls come from the owner; nothing is inferred.
#[derive(Clone, Copy)]
pub struct CombatPhaseView<'a> {
    pub art: CombatArt,
    pub chapter: &'a str,
    pub title: &'a str,
    pub location: &'a str,
    pub narration: &'a str,
    pub connection: &'a str,
    pub notice: &'a str,
    pub turn_label: &'a str,
    pub active_actor: Option<&'a str>,
    pub actors: &'a [CombatActor<'a>],
    pub action_heading: &'a str,
    pub action_empty: &'a str,
    pub offers: &'a [CombatOffer<'a>],
    pub roll_heading: &'a str,
    pub roll_empty: &'a str,
    pub rolls: &'a [CombatRoll<'a>],
    pub reaction: Option<CombatReaction<'a>>,
    pub draft: Option<CombatDraft<'a>>,
    pub feedback: CombatFeedback<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CombatValidationError {
    Capacity,
    EmptyText,
    DuplicateKey,
    UnknownActiveActor,
    InvalidPortrait,
    Disposed,
}
impl fmt::Display for CombatValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Capacity => "combat presentation exceeds owner limits",
            Self::EmptyText => "combat presentation requires nonempty text",
            Self::DuplicateKey => "combat presentation keys repeat",
            Self::UnknownActiveActor => "active actor is absent from supplied initiative",
            Self::InvalidPortrait => "portrait token must be 1-32 letters, digits or hyphens",
            Self::Disposed => "combat presentation scope is disposed",
        })
    }
}
impl std::error::Error for CombatValidationError {}

impl CombatPhaseView<'_> {
    pub fn validate(&self, limits: CombatLimits) -> Result<(), CombatValidationError> {
        if self.actors.len() > limits.max_actors
            || self.offers.len() > limits.max_offers
            || self.rolls.len() > limits.max_rolls
        {
            return Err(CombatValidationError::Capacity);
        }
        let mut total = 0usize;
        let mut text = |value: &str| {
            if value.trim().is_empty() {
                return Err(CombatValidationError::EmptyText);
            }
            if value.len() > limits.max_text_bytes {
                return Err(CombatValidationError::Capacity);
            }
            total = total
                .checked_add(value.len())
                .ok_or(CombatValidationError::Capacity)?;
            if total > limits.max_total_text_bytes {
                return Err(CombatValidationError::Capacity);
            }
            Ok(())
        };
        for value in [
            self.chapter,
            self.title,
            self.location,
            self.narration,
            self.connection,
            self.notice,
            self.turn_label,
            self.action_heading,
            self.action_empty,
            self.roll_heading,
            self.roll_empty,
        ] {
            text(value)?;
        }
        let mut keys = BTreeSet::new();
        for actor in self.actors {
            for value in [actor.key, actor.name, actor.initiative, actor.status] {
                text(value)?;
            }
            if !keys.insert(actor.key) {
                return Err(CombatValidationError::DuplicateKey);
            }
            if actor.resources.len() > limits.max_resources_per_actor {
                return Err(CombatValidationError::Capacity);
            }
            for resource in actor.resources {
                text(resource.label)?;
                text(resource.value)?;
            }
            if let Some(portrait) = actor.portrait
                && (portrait.is_empty()
                    || portrait.len() > 32
                    || !portrait
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
            {
                return Err(CombatValidationError::InvalidPortrait);
            }
        }
        if self.active_actor.is_some_and(|key| !keys.contains(key)) {
            return Err(CombatValidationError::UnknownActiveActor);
        }
        keys.clear();
        for offer in self.offers {
            for value in [
                offer.key,
                offer.offer.as_str(),
                offer.label,
                offer.explanation,
            ] {
                text(value)?;
            }
            if let Some(option) = offer.option {
                text(option.as_str())?;
            }
            if !keys.insert(offer.key) {
                return Err(CombatValidationError::DuplicateKey);
            }
        }
        keys.clear();
        for roll in self.rolls {
            for value in [
                roll.key,
                roll.label,
                roll.value,
                roll.explanation,
                roll.source,
            ] {
                text(value)?;
            }
            if let Some(verdict) = &roll.verdict {
                text(verdict.label)?;
            }
            if !keys.insert(roll.key) {
                return Err(CombatValidationError::DuplicateKey);
            }
        }
        if let Some(reaction) = &self.reaction {
            for value in [reaction.heading, reaction.timing, reaction.explanation] {
                text(value)?;
            }
        }
        if let Some(draft) = &self.draft {
            text(draft.key)?;
            text(draft.label)?;
            if let CombatFeedback::Pending(value) | CombatFeedback::Refused(value) = draft.feedback
            {
                text(value)?;
            }
        }
        if let CombatFeedback::Pending(value) | CombatFeedback::Refused(value) = self.feedback {
            text(value)?;
        }
        Ok(())
    }
}

/// Canonical binding/revision admission without retaining a second game snapshot.
/// Authorized scope replacement requires disposal and a fresh mounting owner.
pub struct CombatPhaseState {
    revisions: Option<ViewStore<()>>,
    limits: CombatLimits,
}
impl CombatPhaseState {
    pub fn new(binding: ClientBindingId, limits: CombatLimits) -> Self {
        Self {
            revisions: Some(ViewStore::new(binding)),
            limits,
        }
    }
    pub fn accept(
        &mut self,
        binding: ClientBindingId,
        revision: SessionRevision,
        view: &CombatPhaseView<'_>,
    ) -> Result<ViewAcceptance, CombatValidationError> {
        let revisions = self
            .revisions
            .as_mut()
            .ok_or(CombatValidationError::Disposed)?;
        view.validate(self.limits)?;
        Ok(revisions.accept(binding, revision, ()))
    }
    pub fn dispose(&mut self) {
        self.revisions = None;
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::{
        ActionView, ControlError, ControlledAction, ControlledTextInput, DraftUpdate,
        InputFeedback, TextInputView, UiError,
    };
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
        rc::Rc,
    };
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{Document, Element, Event, EventTarget, HtmlElement};

    #[derive(Debug)]
    pub enum CombatError {
        InvalidView(CombatValidationError),
        Dom(UiError),
        Control(ControlError),
        CallbackAlreadyRegistered,
        Disposed,
    }
    impl fmt::Display for CombatError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidView(error) => fmt::Display::fmt(error, formatter),
                Self::Dom(error) => fmt::Display::fmt(error, formatter),
                Self::Control(error) => fmt::Display::fmt(error, formatter),
                Self::CallbackAlreadyRegistered => {
                    formatter.write_str("combat callback is already registered")
                }
                Self::Disposed => formatter.write_str("combat surface is disposed"),
            }
        }
    }
    impl std::error::Error for CombatError {}
    impl From<wasm_bindgen::JsValue> for CombatError {
        fn from(error: wasm_bindgen::JsValue) -> Self {
            Self::Dom(UiError::Browser(error))
        }
    }
    impl From<ControlError> for CombatError {
        fn from(error: ControlError) -> Self {
            Self::Control(error)
        }
    }
    impl From<CombatValidationError> for CombatError {
        fn from(error: CombatValidationError) -> Self {
            Self::InvalidView(error)
        }
    }

    type IntentCallback = Box<dyn FnMut(CombatIntent)>;
    struct Dispatch {
        active: bool,
        intents: BTreeMap<String, CombatIntent>,
        callback: Option<IntentCallback>,
        registered: bool,
    }
    struct ActorNodes {
        root: Element,
        name: Element,
        initiative: Element,
        status: Element,
        hit_points: Element,
        hit_points_text: Element,
        resources: Element,
    }
    struct RollNodes {
        root: Element,
        label: Element,
        value: Element,
        verdict: Element,
        explanation: Element,
        source: Element,
    }
    struct OfferNodes {
        root: Element,
        control: ControlledAction,
        explanation: Element,
    }
    struct DraftOwner {
        key: String,
        control: Rc<ControlledTextInput>,
    }
    struct SceneImageListener {
        target: EventTarget,
        name: &'static str,
        active: Rc<Cell<bool>>,
        callback: Option<Closure<dyn FnMut(Event)>>,
    }
    impl SceneImageListener {
        fn bind(
            image: &HtmlElement,
            name: &'static str,
            loaded: bool,
            url: &'static str,
            active: &Rc<Cell<bool>>,
        ) -> Result<Self, CombatError> {
            let target: EventTarget = image.clone().into();
            let callback_image = image.clone();
            let callback_active = Rc::clone(active);
            let callback = Closure::wrap(Box::new(move |_: Event| {
                // Every URL owns a distinct image and active generation token.
                // Queued events on a revoked image cannot affect its replacement.
                if callback_active.get()
                    && callback_image.get_attribute("src").as_deref() == Some(url)
                {
                    callback_image.set_hidden(!loaded);
                }
            }) as Box<dyn FnMut(Event)>);
            target.add_event_listener_with_callback(name, callback.as_ref().unchecked_ref())?;
            Ok(Self {
                target,
                name,
                active: Rc::clone(active),
                callback: Some(callback),
            })
        }
        fn dispose(&mut self) -> Result<(), CombatError> {
            self.active.set(false);
            if let Some(callback) = self.callback.as_ref() {
                self.target.remove_event_listener_with_callback(
                    self.name,
                    callback.as_ref().unchecked_ref(),
                )?;
            }
            self.callback = None;
            Ok(())
        }
    }
    impl Drop for SceneImageListener {
        fn drop(&mut self) {
            // A failed browser removal must retain only an inert closure rather
            // than leave a registered callback pointing at freed WASM code.
            if self.dispose().is_err()
                && let Some(callback) = self.callback.take()
            {
                callback.forget();
            }
        }
    }
    struct SceneImage {
        element: HtmlElement,
        url: &'static str,
        active: Rc<Cell<bool>>,
        listeners: Vec<SceneImageListener>,
        disposed: bool,
    }
    impl SceneImage {
        fn create(document: &Document, art: CombatArt) -> Result<Self, CombatError> {
            let element = document
                .create_element("img")?
                .dyn_into::<HtmlElement>()
                .map_err(|_| CombatError::Dom(UiError::WrongElementType))?;
            element.set_class_name("combat-art");
            // Keep the optional bitmap hidden until its own successful load.
            // The stable host supplies the scene artwork's accessible label.
            element.set_hidden(true);
            element.set_attribute("alt", "")?;
            element.set_attribute("aria-hidden", "true")?;
            element.set_attribute("fetchpriority", "high")?;
            let url = art.asset_path();
            let active = Rc::new(Cell::new(true));
            let mut listeners = Vec::with_capacity(2);
            for (name, loaded) in [("load", true), ("error", false)] {
                listeners.push(SceneImageListener::bind(
                    &element, name, loaded, url, &active,
                )?);
            }
            element.set_attribute("src", url)?;
            Ok(Self {
                element,
                url,
                active,
                listeners,
                disposed: false,
            })
        }
        fn dispose(&mut self) -> Result<(), CombatError> {
            if self.disposed {
                return Ok(());
            }
            self.disposed = true;
            self.active.set(false);
            self.element.set_hidden(true);
            let mut failure = None;
            for listener in &mut self.listeners {
                if let Err(error) = listener.dispose()
                    && failure.is_none()
                {
                    failure = Some(error);
                }
            }
            if let Err(error) = self.element.remove_attribute("src")
                && failure.is_none()
            {
                failure = Some(error.into());
            }
            if let Some(parent) = self.element.parent_node()
                && let Err(error) = parent.remove_child(&self.element)
                && failure.is_none()
            {
                failure = Some(error.into());
            }
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }
    impl Drop for SceneImage {
        fn drop(&mut self) {
            // Explicit scope disposal reports cleanup errors. Drop still hides
            // the bitmap and fences all callbacks if DOM cleanup cannot finish.
            if self.dispose().is_err() {
                self.element.set_hidden(true);
            }
        }
    }
    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
    ) -> Result<Element, CombatError> {
        let element = document.create_element(tag)?;
        element.set_class_name(class);
        parent.append_child(&element)?;
        Ok(element)
    }
    fn set_text(node: &Element, text: &str) {
        if node.text_content().as_deref() != Some(text) {
            node.set_text_content(Some(text));
        }
    }
    fn place(parent: &Element, node: &Element, position: usize) -> Result<(), CombatError> {
        let mut before = parent.first_child();
        for _ in 0..position {
            before = before.and_then(|node| node.next_sibling());
        }
        if !before
            .as_ref()
            .is_some_and(|before| before.is_same_node(Some(node)))
        {
            parent.insert_before(node, before.as_ref())?;
        }
        Ok(())
    }
    fn feedback(value: CombatFeedback<'_>) -> InputFeedback<'_> {
        match value {
            CombatFeedback::None => InputFeedback::None,
            CombatFeedback::Pending(text) => InputFeedback::Pending(text),
            CombatFeedback::Refused(text) => InputFeedback::Rejected(text),
        }
    }

    fn publish_hit_points(
        bar: &Element,
        text: &Element,
        actor: &CombatActor<'_>,
    ) -> Result<(), CombatError> {
        let Some(points) = actor.hit_points else {
            bar.set_attribute("hidden", "")?;
            return Ok(());
        };
        bar.remove_attribute("hidden")?;
        // Presentation ratio only: the supplied numbers are shown unchanged.
        let percent = if points.maximum == 0 {
            0
        } else {
            u64::from(points.current.min(points.maximum)) * 100 / u64::from(points.maximum)
        };
        bar.set_attribute("style", &format!("--hp:{percent}%"))?;
        bar.set_attribute("aria-valuemin", "0")?;
        bar.set_attribute("aria-valuemax", &points.maximum.to_string())?;
        bar.set_attribute("aria-valuenow", &points.current.to_string())?;
        bar.set_attribute(
            "data-state",
            if points.current == 0 {
                "down"
            } else if percent <= 35 {
                "low"
            } else {
                "healthy"
            },
        )?;
        set_text(text, &format!("{} / {} HP", points.current, points.maximum));
        Ok(())
    }

    /// One mounted cinematic phase. Stable keyed controls retain focus; accepted
    /// snapshots invalidate removed offers before callback dispatch. No timers,
    /// geometry, dice generation, outcome calculations or game clock exist here.
    pub struct CombatPhaseSurface {
        document: Document,
        root: Element,
        art_host: Element,
        art: RefCell<Option<SceneImage>>,
        chapter: Element,
        title: Element,
        location: Element,
        narration: Element,
        connection: Element,
        notice: Element,
        turn: Element,
        turn_badge: Element,
        actors_root: Element,
        actors: RefCell<BTreeMap<String, ActorNodes>>,
        action_heading: Element,
        action_empty: Element,
        offers_root: Element,
        offers: RefCell<BTreeMap<String, OfferNodes>>,
        roll_heading: Element,
        roll_empty: Element,
        rolls_root: Element,
        rolls: RefCell<BTreeMap<String, RollNodes>>,
        reaction: Element,
        reaction_heading: Element,
        reaction_timing: Element,
        reaction_explanation: Element,
        draft_root: Element,
        draft_identifier: String,
        draft: Rc<RefCell<Option<DraftOwner>>>,
        feedback: Element,
        dispatch: Rc<RefCell<Dispatch>>,
        state: RefCell<CombatPhaseState>,
        disposed: Cell<bool>,
    }
    impl CombatPhaseSurface {
        pub fn create(
            document: &Document,
            draft_identifier: &str,
            binding: ClientBindingId,
            revision: SessionRevision,
            view: &CombatPhaseView<'_>,
            limits: CombatLimits,
        ) -> Result<Self, CombatError> {
            view.validate(limits)?;
            if draft_identifier.is_empty()
                || draft_identifier.len() > 128
                || !draft_identifier
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            {
                return Err(ControlError::InvalidId.into());
            }
            let root = document.create_element("section")?;
            root.set_class_name("df-combat");
            let style = child(document, &root, "style", "")?;
            style.set_text_content(Some(crate::combat_phase_theme::STYLES));
            let art_host = child(document, &root, "div", "combat-art-host")?;
            art_host.set_attribute("role", "img")?;
            let header = child(document, &root, "header", "combat-topbar")?;
            child(document, &header, "div", "combat-brand")?.set_text_content(Some("DungeonFlux"));
            let location = child(document, &header, "p", "combat-location")?;
            let notice = child(document, &header, "span", "combat-notice")?;
            let stage = child(document, &root, "div", "combat-stage")?;
            let story = child(document, &stage, "div", "combat-story")?;
            let chapter = child(document, &story, "p", "combat-overline")?;
            let title = child(document, &story, "h1", "")?;
            let turn_badge = child(document, &story, "p", "combat-turn-badge")?;
            let narration = child(document, &story, "p", "combat-narration")?;
            let initiative = child(document, &stage, "aside", "combat-initiative")?;
            let turn = child(document, &initiative, "h2", "combat-overline")?;
            let actors_root = child(document, &initiative, "ol", "combat-actors")?;
            let lower = child(document, &root, "div", "combat-lower")?;
            let action_rail = child(document, &lower, "section", "combat-action-rail")?;
            let action_heading = child(document, &action_rail, "h2", "")?;
            let action_empty = child(document, &action_rail, "p", "combat-muted")?;
            let offers_root = child(document, &action_rail, "div", "combat-offers")?;
            let reaction = child(document, &action_rail, "section", "combat-reaction")?;
            let reaction_heading = child(document, &reaction, "h3", "")?;
            let reaction_timing = child(document, &reaction, "p", "combat-timing")?;
            let reaction_explanation = child(document, &reaction, "p", "combat-muted")?;
            let draft_root = child(document, &action_rail, "div", "combat-draft")?;
            let feedback = child(document, &action_rail, "p", "combat-feedback")?;
            feedback.set_attribute("role", "status")?;
            feedback.set_attribute("aria-live", "polite")?;
            let roll_panel = child(document, &lower, "section", "combat-roll-panel")?;
            let roll_heading = child(document, &roll_panel, "h2", "")?;
            let roll_empty = child(document, &roll_panel, "p", "combat-muted")?;
            let rolls_root = child(document, &roll_panel, "ol", "combat-rolls")?;
            let connection = child(document, &root, "footer", "combat-connection")?;
            let surface = Self {
                document: document.clone(),
                root,
                art_host,
                art: RefCell::new(None),
                chapter,
                title,
                location,
                narration,
                connection,
                notice,
                turn,
                turn_badge,
                actors_root,
                actors: RefCell::new(BTreeMap::new()),
                action_heading,
                action_empty,
                offers_root,
                offers: RefCell::new(BTreeMap::new()),
                roll_heading,
                roll_empty,
                rolls_root,
                rolls: RefCell::new(BTreeMap::new()),
                reaction,
                reaction_heading,
                reaction_timing,
                reaction_explanation,
                draft_root,
                draft_identifier: draft_identifier.to_owned(),
                draft: Rc::new(RefCell::new(None)),
                feedback,
                dispatch: Rc::new(RefCell::new(Dispatch {
                    active: true,
                    intents: BTreeMap::new(),
                    callback: None,
                    registered: false,
                })),
                state: RefCell::new(CombatPhaseState::new(binding, limits)),
                disposed: Cell::new(false),
            };
            surface.update(binding, revision, view)?;
            Ok(surface)
        }
        pub fn root(&self) -> &Element {
            &self.root
        }
        pub fn draft_input(&self) -> Option<Rc<ControlledTextInput>> {
            self.draft
                .borrow()
                .as_ref()
                .map(|owner| Rc::clone(&owner.control))
        }
        pub fn on_intent(
            &self,
            callback: impl FnMut(CombatIntent) + 'static,
        ) -> Result<(), CombatError> {
            if self.disposed.get() {
                return Err(CombatError::Disposed);
            }
            let mut dispatch = self.dispatch.borrow_mut();
            if dispatch.registered {
                return Err(CombatError::CallbackAlreadyRegistered);
            }
            dispatch.callback = Some(Box::new(callback));
            dispatch.registered = true;
            Ok(())
        }
        pub fn update(
            &self,
            binding: ClientBindingId,
            revision: SessionRevision,
            view: &CombatPhaseView<'_>,
        ) -> Result<ViewAcceptance, CombatError> {
            if self.disposed.get() {
                return Err(CombatError::Disposed);
            }
            let outcome = self.state.borrow_mut().accept(binding, revision, view)?;
            if outcome != ViewAcceptance::Applied {
                return Ok(outcome);
            }
            // Fence all earlier controls before touching the DOM. A failed render
            // returns a typed error and cannot send a partially updated selection.
            self.dispatch.borrow_mut().intents.clear();
            self.root.set_attribute("data-combat-render", "pending")?;
            self.publish(view)?;
            let mut dispatch = self.dispatch.borrow_mut();
            for offer in view
                .offers
                .iter()
                .filter(|offer| offer.enabled && !offer.pending)
            {
                dispatch.intents.insert(
                    offer.key.to_owned(),
                    CombatIntent {
                        binding,
                        revision,
                        kind: offer.kind,
                        offer: offer.offer.clone(),
                        option: offer.option.cloned(),
                        draft: None,
                    },
                );
            }
            self.root.set_attribute(
                "data-combat-revision",
                &format!("{}:{}", revision.epoch().get(), revision.sequence()),
            )?;
            self.root.set_attribute("data-combat-render", "ready")?;
            Ok(outcome)
        }
        fn publish(&self, view: &CombatPhaseView<'_>) -> Result<(), CombatError> {
            self.publish_art(view.art)?;
            for (node, value) in [
                (&self.chapter, view.chapter),
                (&self.title, view.title),
                (&self.location, view.location),
                (&self.narration, view.narration),
                (&self.connection, view.connection),
                (&self.notice, view.notice),
                (&self.turn, view.turn_label),
                (&self.turn_badge, view.turn_label),
                (&self.action_heading, view.action_heading),
                (&self.roll_heading, view.roll_heading),
            ] {
                set_text(node, value);
            }
            set_text(
                &self.action_empty,
                if view.offers.is_empty() {
                    view.action_empty
                } else {
                    ""
                },
            );
            set_text(
                &self.roll_empty,
                if view.rolls.is_empty() {
                    view.roll_empty
                } else {
                    ""
                },
            );
            self.publish_actors(view)?;
            self.publish_rolls(view)?;
            self.publish_offers(view)?;
            if let Some(reaction) = &view.reaction {
                self.reaction.remove_attribute("hidden")?;
                set_text(&self.reaction_heading, reaction.heading);
                set_text(&self.reaction_timing, reaction.timing);
                set_text(&self.reaction_explanation, reaction.explanation);
            } else {
                self.reaction.set_attribute("hidden", "")?;
                for node in [
                    &self.reaction_heading,
                    &self.reaction_timing,
                    &self.reaction_explanation,
                ] {
                    node.set_text_content(None);
                }
            }
            self.publish_draft(view.draft.as_ref())?;
            let (message, state) = match view.feedback {
                CombatFeedback::None => ("", "none"),
                CombatFeedback::Pending(message) => (message, "pending"),
                CombatFeedback::Refused(message) => (message, "refused"),
            };
            set_text(&self.feedback, message);
            self.feedback.set_attribute("data-state", state)?;
            Ok(())
        }
        fn publish_art(&self, view: CombatArt) -> Result<(), CombatError> {
            self.art_host
                .set_attribute("aria-label", view.description())?;
            let mut art = self.art.borrow_mut();
            if art
                .as_ref()
                .is_some_and(|image| image.url == view.asset_path())
            {
                return Ok(());
            }
            // URL replacement revokes its listeners before admitting a new image.
            // Same-scene updates reuse the bounded owner without another fetch.
            if let Some(mut obsolete) = art.take() {
                obsolete.dispose()?;
            }
            let image = SceneImage::create(&self.document, view)?;
            self.art_host.append_child(&image.element)?;
            *art = Some(image);
            Ok(())
        }
        fn publish_actors(&self, view: &CombatPhaseView<'_>) -> Result<(), CombatError> {
            let mut actors = self.actors.borrow_mut();
            let obsolete: Vec<_> = actors
                .keys()
                .filter(|key| !view.actors.iter().any(|actor| actor.key == key.as_str()))
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(node) = actors.remove(&key) {
                    self.actors_root.remove_child(&node.root)?;
                }
            }
            for (position, actor) in view.actors.iter().enumerate() {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    actors.entry(actor.key.to_owned())
                {
                    let root = self.document.create_element("li")?;
                    root.set_class_name("combat-actor");
                    let initiative =
                        child(&self.document, &root, "span", "combat-initiative-value")?;
                    let body = child(&self.document, &root, "div", "combat-actor-body")?;
                    let name = child(&self.document, &body, "h3", "")?;
                    let status = child(&self.document, &body, "p", "combat-muted")?;
                    let hit_points = child(&self.document, &body, "div", "combat-hp")?;
                    hit_points.set_attribute("role", "meter")?;
                    hit_points.set_attribute("aria-label", "Hit points")?;
                    child(&self.document, &hit_points, "span", "combat-hp-fill")?;
                    let hit_points_text =
                        child(&self.document, &hit_points, "span", "combat-hp-text")?;
                    let resources = child(&self.document, &body, "dl", "combat-resources")?;
                    entry.insert(ActorNodes {
                        root,
                        name,
                        initiative,
                        status,
                        hit_points,
                        hit_points_text,
                        resources,
                    });
                }
                if let Some(nodes) = actors.get(actor.key) {
                    set_text(&nodes.name, actor.name);
                    set_text(&nodes.initiative, actor.initiative);
                    set_text(&nodes.status, actor.status);
                    nodes.root.set_attribute(
                        "data-active",
                        if view.active_actor == Some(actor.key) {
                            "true"
                        } else {
                            "false"
                        },
                    )?;
                    nodes.root.set_attribute("data-actor-key", actor.key)?;
                    match actor.portrait {
                        Some(portrait) => nodes.root.set_attribute("data-portrait", portrait)?,
                        None => nodes.root.remove_attribute("data-portrait")?,
                    }
                    publish_hit_points(&nodes.hit_points, &nodes.hit_points_text, actor)?;
                    nodes.resources.set_text_content(None);
                    for resource in actor.resources {
                        child(&self.document, &nodes.resources, "dt", "")?
                            .set_text_content(Some(resource.label));
                        child(&self.document, &nodes.resources, "dd", "")?
                            .set_text_content(Some(resource.value));
                    }
                    place(&self.actors_root, &nodes.root, position)?;
                }
            }
            Ok(())
        }
        fn publish_rolls(&self, view: &CombatPhaseView<'_>) -> Result<(), CombatError> {
            let mut rolls = self.rolls.borrow_mut();
            let obsolete: Vec<_> = rolls
                .keys()
                .filter(|key| !view.rolls.iter().any(|roll| roll.key == key.as_str()))
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(nodes) = rolls.remove(&key) {
                    self.rolls_root.remove_child(&nodes.root)?;
                }
            }
            for (position, roll) in view.rolls.iter().enumerate() {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    rolls.entry(roll.key.to_owned())
                {
                    let root = self.document.create_element("li")?;
                    root.set_class_name("combat-roll");
                    let label = child(&self.document, &root, "h3", "")?;
                    let value = child(&self.document, &root, "p", "combat-roll-value")?;
                    let verdict = child(&self.document, &root, "p", "combat-roll-verdict")?;
                    let explanation = child(&self.document, &root, "p", "combat-roll-explanation")?;
                    let source = child(&self.document, &root, "p", "combat-roll-source")?;
                    entry.insert(RollNodes {
                        root,
                        label,
                        value,
                        verdict,
                        explanation,
                        source,
                    });
                }
                if let Some(nodes) = rolls.get(roll.key) {
                    set_text(&nodes.label, roll.label);
                    set_text(&nodes.value, roll.value);
                    nodes.value.set_attribute(
                        "data-nat",
                        match roll.die_face {
                            Some(20) => "20",
                            Some(1) => "1",
                            _ => "mid",
                        },
                    )?;
                    // The resting frame behind the number, small; machine-generated art
                    // URL, so it is rendered but not text-validated with the words.
                    match roll.die_rest {
                        Some(rest) => nodes
                            .value
                            .set_attribute("style", &format!("background-image:url(\"{rest}\")"))?,
                        None => nodes.value.remove_attribute("style")?,
                    }
                    set_text(&nodes.explanation, roll.explanation);
                    set_text(&nodes.source, roll.source);
                    match &roll.verdict {
                        Some(verdict) => {
                            nodes.verdict.remove_attribute("hidden")?;
                            set_text(&nodes.verdict, verdict.label);
                            nodes.verdict.set_attribute(
                                "data-tone",
                                match verdict.tone {
                                    CombatTone::Success => "success",
                                    CombatTone::Failure => "failure",
                                    CombatTone::Neutral => "neutral",
                                },
                            )?;
                        }
                        None => {
                            nodes.verdict.set_attribute("hidden", "")?;
                            nodes.verdict.set_text_content(None);
                        }
                    }
                    nodes.root.set_attribute("data-roll-key", roll.key)?;
                    place(&self.rolls_root, &nodes.root, position)?;
                }
            }
            Ok(())
        }
        fn publish_offers(&self, view: &CombatPhaseView<'_>) -> Result<(), CombatError> {
            let mut offers = self.offers.borrow_mut();
            let active_element = self.document.active_element();
            let focused_key = offers
                .iter()
                .find(|(_, nodes)| {
                    active_element
                        .as_ref()
                        .is_some_and(|active| active.is_same_node(Some(nodes.control.element())))
                })
                .map(|(key, _)| key.clone());
            let obsolete: Vec<_> = offers
                .keys()
                .filter(|key| !view.offers.iter().any(|offer| offer.key == key.as_str()))
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(nodes) = offers.remove(&key) {
                    nodes.control.dispose()?;
                    self.offers_root.remove_child(&nodes.root)?;
                }
            }
            for (position, offer) in view.offers.iter().enumerate() {
                let action_view = ActionView {
                    label: offer.label,
                    enabled: offer.enabled,
                    pending: offer.pending,
                };
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    offers.entry(offer.key.to_owned())
                {
                    let root = self.document.create_element("div")?;
                    root.set_class_name("combat-offer");
                    let control = ControlledAction::create(&self.document, action_view)?;
                    root.append_child(control.element())?;
                    let explanation = child(&self.document, &root, "p", "combat-muted")?;
                    let dispatch = Rc::clone(&self.dispatch);
                    let drafts = Rc::clone(&self.draft);
                    let key = offer.key.to_owned();
                    control.on_activate(move || {
                        let selected = {
                            let mut owner = dispatch.borrow_mut();
                            if !owner.active {
                                return;
                            }
                            owner.intents.get(&key).cloned().and_then(|intent| {
                                owner.callback.take().map(|callback| (intent, callback))
                            })
                        };
                        if let Some((mut intent, mut callback)) = selected {
                            intent.draft =
                                drafts.borrow().as_ref().map(|owner| owner.control.draft());
                            // No RefCell borrow survives the user callback. It may
                            // synchronously publish a newer snapshot or dispose us.
                            callback(intent);
                            let mut owner = dispatch.borrow_mut();
                            if owner.active {
                                owner.callback = Some(callback);
                            }
                        }
                    })?;
                    entry.insert(OfferNodes {
                        root,
                        control,
                        explanation,
                    });
                }
                if let Some(nodes) = offers.get(offer.key) {
                    nodes.control.update(action_view)?;
                    nodes
                        .control
                        .element()
                        .set_attribute("data-combat-offer", offer.key)?;
                    nodes.root.set_attribute(
                        "data-kind",
                        match offer.kind {
                            CombatOfferKind::Action => "action",
                            CombatOfferKind::Roll => "roll",
                            CombatOfferKind::Reaction => "reaction",
                            CombatOfferKind::Choice { .. } => "choice",
                        },
                    )?;
                    if let CombatOfferKind::Choice { selected } = offer.kind {
                        let aria_state = if selected { "true" } else { "false" };
                        nodes
                            .control
                            .element()
                            .set_attribute("aria-pressed", aria_state)?;
                        nodes.root.set_attribute("data-selected", aria_state)?;
                    } else {
                        nodes.control.element().remove_attribute("aria-pressed")?;
                        nodes.root.remove_attribute("data-selected")?;
                    }
                    set_text(&nodes.explanation, offer.explanation);
                    place(&self.offers_root, &nodes.root, position)?;
                }
            }
            // insertBefore preserves identity but may blur a moved native button.
            // Restore only a still-present, enabled control; no borrow survives
            // focus(), which can synchronously deliver browser focus events.
            let focus_target = focused_key
                .and_then(|key| offers.get(&key))
                .filter(|nodes| !nodes.control.element().disabled())
                .map(|nodes| nodes.control.element().clone());
            drop(offers);
            if let Some(target) = focus_target
                && !self
                    .document
                    .active_element()
                    .is_some_and(|active| active.is_same_node(Some(&target)))
            {
                target.focus()?;
            }
            Ok(())
        }
        fn publish_draft(&self, view: Option<&CombatDraft<'_>>) -> Result<(), CombatError> {
            let mut draft = self.draft.borrow_mut();
            if draft
                .as_ref()
                .is_some_and(|owner| view.is_none_or(|view| view.key != owner.key))
                && let Some(owner) = draft.take()
            {
                owner.control.dispose()?;
            }
            if let Some(view) = view {
                let input_view = TextInputView {
                    label: view.label,
                    enabled: view.enabled,
                    feedback: feedback(view.feedback),
                };
                if draft.is_none() {
                    let control = Rc::new(ControlledTextInput::create(
                        &self.document,
                        &self.draft_identifier,
                        input_view,
                        "",
                    )?);
                    self.draft_root.append_child(control.root())?;
                    *draft = Some(DraftOwner {
                        key: view.key.to_owned(),
                        control,
                    });
                } else if let Some(owner) = draft.as_ref() {
                    owner.control.update(input_view, DraftUpdate::Preserve)?;
                }
            }
            Ok(())
        }
        /// Repeatable terminal disposal. Callback fencing precedes DOM cleanup;
        /// retained detached buttons cannot send an obsolete intent.
        pub fn dispose(&self) -> Result<(), CombatError> {
            self.disposed.set(true);
            self.state.borrow_mut().dispose();
            {
                let mut dispatch = self.dispatch.borrow_mut();
                dispatch.active = false;
                dispatch.intents.clear();
                dispatch.callback = None;
            }
            // Attempt every cleanup even when one DOM operation fails. Controls
            // clear private state and fence listeners before attempting removal.
            let mut failure = None;
            for nodes in self.offers.borrow().values() {
                if let Err(error) = nodes.control.dispose()
                    && failure.is_none()
                {
                    failure = Some(CombatError::Control(error));
                }
            }
            self.offers.borrow_mut().clear();
            if let Some(owner) = self.draft.borrow_mut().take()
                && let Err(error) = owner.control.dispose()
                && failure.is_none()
            {
                failure = Some(CombatError::Control(error));
            }
            self.actors.borrow_mut().clear();
            self.rolls.borrow_mut().clear();
            if let Some(mut art) = self.art.borrow_mut().take()
                && let Err(error) = art.dispose()
                && failure.is_none()
            {
                failure = Some(error);
            }
            self.root.set_text_content(None);
            if let Some(parent) = self.root.parent_node()
                && let Err(error) = parent.remove_child(&self.root)
                && failure.is_none()
            {
                failure = Some(error.into());
            }
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }
    impl Drop for CombatPhaseSurface {
        fn drop(&mut self) {
            // Drop is terminal even when draft_input() has external Rc owners.
            // Explicit dispose reports DOM errors; Drop cannot return them. Its
            // cleanup still attempts every resource and clears retained private
            // draft/callback state before any fallible DOM removal.
            if self.dispose().is_err() {
                self.root.set_text_content(None);
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::{CombatError, CombatPhaseSurface};

#[cfg(test)]
mod tests {
    use super::*;
    use df_types::RecoveryEpoch;

    fn limits() -> CombatLimits {
        CombatLimits {
            max_actors: 8,
            max_offers: 12,
            max_rolls: 8,
            max_resources_per_actor: 8,
            max_text_bytes: 1024,
            max_total_text_bytes: 8192,
        }
    }
    fn view<'a>(
        actors: &'a [CombatActor<'a>],
        offers: &'a [CombatOffer<'a>],
        rolls: &'a [CombatRoll<'a>],
    ) -> CombatPhaseView<'a> {
        CombatPhaseView {
            art: CombatArt::Harbor,
            chapter: "Encounter",
            title: "Harbor",
            location: "Docks",
            narration: "Lanterns sway",
            connection: "Connected",
            notice: "Synthetic fixture",
            turn_label: "Round supplied by server",
            active_actor: actors.first().map(|actor| actor.key),
            actors,
            action_heading: "Your choices",
            action_empty: "No current offers",
            offers,
            roll_heading: "Resolved rolls",
            roll_empty: "No resolved rolls",
            rolls,
            reaction: None,
            draft: None,
            feedback: CombatFeedback::None,
        }
    }
    #[test]
    fn borrowed_facts_preserve_exact_order_resources_and_non_numeric_roll_values() {
        let resources = [CombatResource {
            label: "Movement",
            value: "Not supplied",
        }];
        let actors = [
            CombatActor {
                key: "b",
                name: "B",
                initiative: "Tied · first",
                status: "Active actor",
                resources: &resources,
                hit_points: None,
                portrait: None,
            },
            CombatActor {
                key: "a",
                name: "A",
                initiative: "Tied · second",
                status: "Waiting",
                resources: &[],
                hit_points: None,
                portrait: None,
            },
        ];
        let rolls = [CombatRoll {
            key: "r",
            label: "Saving throw",
            value: "Outcome withheld",
            explanation: "Server is resolving the save",
            source: "Supplied source",
            verdict: None,
            die_face: None,
            die_rest: None,
        }];
        let supplied = view(&actors, &[], &rolls);
        assert_eq!(supplied.validate(limits()), Ok(()));
        assert!(std::ptr::eq(supplied.actors, actors.as_slice()));
        assert_eq!(supplied.actors.first().map(|actor| actor.key), Some("b"));
        assert_eq!(
            supplied
                .actors
                .first()
                .and_then(|actor| actor.resources.first())
                .map(|resource| resource.value),
            Some("Not supplied")
        );
        assert_eq!(
            supplied.rolls.first().map(|roll| roll.value),
            Some("Outcome withheld")
        );
    }
    #[test]
    fn malformed_or_oversized_views_are_rejected_without_changing_the_watermark() {
        let binding = ClientBindingId::from_bytes(&[1; 16]).unwrap();
        let epoch = RecoveryEpoch::new(1).unwrap();
        let mut state = CombatPhaseState::new(binding, limits());
        let mut supplied = view(&[], &[], &[]);
        supplied.active_actor = Some("not-present");
        assert_eq!(
            state.accept(binding, SessionRevision::new(epoch, 9), &supplied),
            Err(CombatValidationError::UnknownActiveActor)
        );
        supplied.active_actor = None;
        assert_eq!(
            state.accept(binding, SessionRevision::new(epoch, 1), &supplied),
            Ok(ViewAcceptance::Applied)
        );
        supplied.title = " ";
        assert_eq!(
            supplied.validate(limits()),
            Err(CombatValidationError::EmptyText)
        );
        supplied.title = "Title";
        assert_eq!(
            supplied.validate(CombatLimits {
                max_total_text_bytes: 10,
                ..limits()
            }),
            Err(CombatValidationError::Capacity)
        );
    }
    #[test]
    fn portrait_tokens_and_verdict_labels_are_bounded_presentation_text() {
        let mut actors = [CombatActor {
            key: "a",
            name: "A",
            initiative: "A",
            status: "Ready",
            resources: &[],
            hit_points: Some(CombatHitPoints {
                current: 3,
                maximum: 13,
            }),
            portrait: Some("hero-1"),
        }];
        assert_eq!(view(&actors, &[], &[]).validate(limits()), Ok(()));
        actors[0].portrait = Some("not a token");
        assert_eq!(
            view(&actors, &[], &[]).validate(limits()),
            Err(CombatValidationError::InvalidPortrait)
        );
        let rolls = [CombatRoll {
            key: "r",
            label: "Attack",
            value: "17",
            explanation: "17 +5 = 22 vs AC 12",
            source: "7 damage",
            verdict: Some(CombatVerdict {
                label: " ",
                tone: CombatTone::Success,
            }),
            die_face: Some(17),
            die_rest: None,
        }];
        assert_eq!(
            view(&[], &[], &rolls).validate(limits()),
            Err(CombatValidationError::EmptyText)
        );
    }
    #[test]
    fn duplicate_offer_and_roll_keys_cannot_ambiguously_route_a_selection() {
        let label = RevisionLabel::new(Some("server-offer-1")).unwrap();
        let offers = [
            CombatOffer {
                key: "same",
                offer: &label,
                option: None,
                kind: CombatOfferKind::Roll,
                label: "Request roll",
                explanation: "Draw happens on server",
                enabled: true,
                pending: false,
            },
            CombatOffer {
                key: "same",
                offer: &label,
                option: None,
                kind: CombatOfferKind::Action,
                label: "Choose action",
                explanation: "Server validates",
                enabled: false,
                pending: false,
            },
        ];
        assert_eq!(
            view(&[], &offers, &[]).validate(limits()),
            Err(CombatValidationError::DuplicateKey)
        );
        let rolls = [
            CombatRoll {
                key: "same",
                label: "Check",
                value: "17",
                explanation: "13 + 4",
                source: "Server source",
                verdict: None,
                die_face: Some(17),
                die_rest: None,
            },
            CombatRoll {
                key: "same",
                label: "Save",
                value: "14",
                explanation: "12 + 2",
                source: "Server source",
                verdict: None,
                die_face: Some(14),
                die_rest: None,
            },
        ];
        assert_eq!(
            view(&[], &[], &rolls).validate(limits()),
            Err(CombatValidationError::DuplicateKey)
        );
    }
    #[test]
    fn stale_results_wrong_bindings_and_disposed_scopes_cannot_replace_current_phase() {
        let binding = ClientBindingId::from_bytes(&[1; 16]).unwrap();
        let wrong = ClientBindingId::from_bytes(&[2; 16]).unwrap();
        let current = SessionRevision::new(RecoveryEpoch::new(2).unwrap(), 1);
        let stale = SessionRevision::new(RecoveryEpoch::new(1).unwrap(), 99);
        let mut state = CombatPhaseState::new(binding, limits());
        let supplied = view(&[], &[], &[]);
        assert_eq!(
            state.accept(binding, current, &supplied),
            Ok(ViewAcceptance::Applied)
        );
        assert_eq!(
            state.accept(binding, stale, &supplied),
            Ok(ViewAcceptance::Stale { current })
        );
        assert_eq!(
            state.accept(binding, current, &supplied),
            Ok(ViewAcceptance::Duplicate { current })
        );
        assert_eq!(
            state.accept(wrong, current.next_sequence().unwrap(), &supplied),
            Ok(ViewAcceptance::WrongBinding)
        );
        state.dispose();
        state.dispose();
        assert_eq!(
            state.accept(binding, current, &supplied),
            Err(CombatValidationError::Disposed)
        );
    }
}
