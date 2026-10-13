use std::{collections::BTreeSet, fmt};

/// Closed illustrative presentation artwork, including concept-inspired generated art.
/// These portraits illustrate a choice; they do not imply a generated player portrait.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharacterPortrait {
    Narrator,
    Vell,
}

impl CharacterPortrait {
    pub const fn asset_path(self) -> &'static str {
        match self {
            Self::Narrator => "assets/ui/portraits/lantern-keeper-generated-v2.png",
            Self::Vell => "assets/concept-art/vell-avatar.webp",
        }
    }
    pub const fn description(self) -> &'static str {
        match self {
            Self::Narrator => "AI-generated concept-inspired portrait of a hooded storyteller",
            Self::Vell => "Original concept portrait of Vell the barkeeper",
        }
    }
}

/// Local resource bounds, unrelated to the legality of a character build.
#[derive(Clone, Copy, Debug)]
pub struct CharacterLimits {
    pub max_groups: usize,
    pub max_options: usize,
    pub max_facts: usize,
    pub max_actions: usize,
    pub max_text_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterOption {
    pub id: String,
    pub label: String,
    pub description: String,
    pub enabled: bool,
    /// Audience-safe explanation of availability supplied by the server.
    pub availability: String,
    pub portrait: Option<CharacterPortrait>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterGroup {
    pub id: String,
    pub label: String,
    pub description: String,
    pub options: Vec<CharacterOption>,
    pub selected: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterFact {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharacterActionKind {
    SubmitDraft,
    MarkReady,
    Reopen,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterAction {
    pub id: String,
    pub kind: CharacterActionKind,
    pub label: String,
    pub enabled: bool,
}

/// Readiness is always a reported view value, never inferred from clicking a button.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharacterStatus {
    Editing,
    Pending,
    Rejected,
    Ready,
    Locked,
}

impl CharacterStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Editing => "Your choices",
            Self::Pending => "Server validation pending",
            Self::Rejected => "Server validation rejected",
            Self::Ready => "Server reports ready",
            Self::Locked => "Server reports locked",
        }
    }
}

/// Presentation projection, not an RPC or game-model replacement. The mounting
/// owner maps a reviewed audience-safe server view into this bounded projection.
/// Scope changes clear private drafts; revisions increase within a scope. An exact
/// duplicate is idempotent; different content at an equal revision is rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterPhaseView {
    pub generation: u64,
    pub owner_key: String,
    pub revision: u64,
    pub chapter: String,
    pub title: String,
    pub description: String,
    pub connection: String,
    pub status: CharacterStatus,
    pub status_message: String,
    pub editable: bool,
    pub name: String,
    pub flavor: String,
    /// Offered cosmetic draft. Absence removes appearance controls and local values.
    pub appearance: Option<CharacterAppearanceDraft>,
    pub portrait: Option<CharacterPortrait>,
    pub groups: Vec<CharacterGroup>,
    /// Complete server-provided summary values; no local stat calculations.
    pub facts: Vec<CharacterFact>,
    pub actions: Vec<CharacterAction>,
}

/// Visual details for a reference sheet created after character confirmation.
/// These descriptions do not grant rules, equipment, or inventory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterAppearanceDraft {
    pub features: String,
    pub outfit: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterChoice {
    pub group_id: String,
    pub option_id: String,
}

/// Caller submits these advertised identifiers/drafts through its authorized
/// command boundary. Receipt, transport and server validation remain caller-owned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterSubmission {
    pub generation: u64,
    pub owner_key: String,
    pub revision: u64,
    pub action_id: String,
    pub kind: CharacterActionKind,
    pub choices: Vec<CharacterChoice>,
    pub name: String,
    pub flavor: String,
    pub appearance: Option<CharacterAppearanceDraft>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharacterValidationError {
    ResourceLimit,
    InvalidText,
    DuplicateId,
    UnknownSelection,
    Unavailable,
    StaleRevision,
    Disposed,
}

impl fmt::Display for CharacterValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResourceLimit => "character presentation exceeds owner resource limits",
            Self::InvalidText => "character presentation text is invalid",
            Self::DuplicateId => "character presentation identifiers repeat",
            Self::UnknownSelection => "character selection is not an advertised option",
            Self::Unavailable => "character control is unavailable",
            Self::StaleRevision => "character presentation revision is stale",
            Self::Disposed => "character presentation is disposed",
        })
    }
}
impl std::error::Error for CharacterValidationError {}

fn validate_text(
    value: &str,
    maximum: usize,
    empty_allowed: bool,
) -> Result<(), CharacterValidationError> {
    if value.len() > maximum {
        return Err(CharacterValidationError::ResourceLimit);
    }
    if (!empty_allowed && value.trim().is_empty())
        || value.chars().any(|character| character.is_control())
    {
        return Err(CharacterValidationError::InvalidText);
    }
    Ok(())
}

impl CharacterPhaseView {
    pub fn validate(&self, limits: CharacterLimits) -> Result<(), CharacterValidationError> {
        if self.groups.len() > limits.max_groups
            || self.facts.len() > limits.max_facts
            || self.actions.len() > limits.max_actions
        {
            return Err(CharacterValidationError::ResourceLimit);
        }
        for text in [
            &self.owner_key,
            &self.chapter,
            &self.title,
            &self.description,
            &self.connection,
            &self.status_message,
        ] {
            validate_text(text, limits.max_text_bytes, false)?;
        }
        for draft in [&self.name, &self.flavor] {
            validate_text(draft, limits.max_text_bytes, true)?;
            if draft.encode_utf16().count() > crate::MAX_DRAFT_UTF16_UNITS {
                return Err(CharacterValidationError::ResourceLimit);
            }
        }
        if let Some(appearance) = &self.appearance {
            for draft in [&appearance.features, &appearance.outfit] {
                validate_text(draft, limits.max_text_bytes, true)?;
                if draft.encode_utf16().count() > crate::MAX_DRAFT_UTF16_UNITS {
                    return Err(CharacterValidationError::ResourceLimit);
                }
            }
        }
        let mut groups = BTreeSet::new();
        let mut option_count = 0usize;
        for group in &self.groups {
            for text in [&group.id, &group.label, &group.description] {
                validate_text(text, limits.max_text_bytes, false)?;
            }
            if !groups.insert(&group.id) {
                return Err(CharacterValidationError::DuplicateId);
            }
            option_count = option_count
                .checked_add(group.options.len())
                .ok_or(CharacterValidationError::ResourceLimit)?;
            if option_count > limits.max_options {
                return Err(CharacterValidationError::ResourceLimit);
            }
            let mut options = BTreeSet::new();
            for option in &group.options {
                for text in [&option.id, &option.label, &option.availability] {
                    validate_text(text, limits.max_text_bytes, false)?;
                }
                // An option may have no description of its own; the card then shows none.
                validate_text(&option.description, limits.max_text_bytes, true)?;
                if !options.insert(&option.id) {
                    return Err(CharacterValidationError::DuplicateId);
                }
            }
            if group
                .selected
                .as_ref()
                .is_some_and(|selected| !options.contains(selected))
            {
                return Err(CharacterValidationError::UnknownSelection);
            }
        }
        for fact in &self.facts {
            validate_text(&fact.label, limits.max_text_bytes, false)?;
            validate_text(&fact.value, limits.max_text_bytes, false)?;
        }
        let mut actions = BTreeSet::new();
        for action in &self.actions {
            validate_text(&action.id, limits.max_text_bytes, false)?;
            validate_text(&action.label, limits.max_text_bytes, false)?;
            if !actions.insert(&action.id) {
                return Err(CharacterValidationError::DuplicateId);
            }
        }
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
struct CharacterDraftState {
    view: CharacterPhaseView,
    choices: std::collections::BTreeMap<String, String>,
    dirty_groups: BTreeSet<String>,
    disposed: bool,
}

#[cfg(any(target_arch = "wasm32", test))]
impl CharacterDraftState {
    fn create(view: &CharacterPhaseView) -> Self {
        Self {
            view: view.clone(),
            choices: view.groups.iter().filter_map(initial_choice).collect(),
            dirty_groups: BTreeSet::new(),
            disposed: false,
        }
    }
    fn editable(&self) -> bool {
        !self.disposed
            && self.view.editable
            && matches!(
                self.view.status,
                CharacterStatus::Editing | CharacterStatus::Rejected
            )
    }
    fn reconcile(&mut self, view: &CharacterPhaseView) -> Result<bool, CharacterValidationError> {
        if self.disposed {
            return Err(CharacterValidationError::Disposed);
        }
        let replaced =
            self.view.generation != view.generation || self.view.owner_key != view.owner_key;
        if !replaced {
            if view.revision < self.view.revision
                || (view.revision == self.view.revision && *view != self.view)
            {
                return Err(CharacterValidationError::StaleRevision);
            }
            if *view == self.view {
                return Ok(false);
            }
        }
        if replaced {
            *self = Self::create(view);
        } else {
            self.choices.retain(|group, option| {
                view.groups.iter().any(|offered| {
                    offered.id == *group
                        && offered.options.iter().any(|candidate| {
                            candidate.id == *option
                                && (candidate.enabled || offered.selected.as_ref() == Some(option))
                        })
                })
            });
            self.dirty_groups
                .retain(|group| self.choices.contains_key(group));
            for group in &view.groups {
                if !self.dirty_groups.contains(&group.id)
                    || matches!(
                        view.status,
                        CharacterStatus::Ready | CharacterStatus::Locked
                    )
                {
                    self.choices.remove(&group.id);
                    if let Some((id, option)) = initial_choice(group) {
                        self.choices.insert(id, option);
                    }
                    self.dirty_groups.remove(&group.id);
                }
            }
            self.view = view.clone();
        }
        Ok(replaced)
    }
    fn choose(&mut self, group: &str, option: &str) -> Result<(), CharacterValidationError> {
        if !self.editable() {
            return Err(CharacterValidationError::Unavailable);
        }
        let offered = self
            .view
            .groups
            .iter()
            .find(|candidate| candidate.id == group)
            .and_then(|candidate| {
                candidate
                    .options
                    .iter()
                    .find(|candidate| candidate.id == option)
            })
            .ok_or(CharacterValidationError::UnknownSelection)?;
        if !offered.enabled {
            return Err(CharacterValidationError::Unavailable);
        }
        self.choices.insert(group.to_owned(), option.to_owned());
        self.dirty_groups.insert(group.to_owned());
        Ok(())
    }
    fn select_only_visible_option(&mut self, group: &str, visible_options: &[String]) {
        if self
            .choices
            .get(group)
            .is_some_and(|selected| !visible_options.contains(selected))
        {
            self.choices.remove(group);
            self.dirty_groups.remove(group);
        }
        if let [option] = visible_options
            && self.view.groups.iter().any(|offered| {
                offered.id == group
                    && offered
                        .options
                        .iter()
                        .any(|candidate| candidate.id == *option && candidate.enabled)
            })
        {
            self.choices.insert(group.to_owned(), option.clone());
            self.dirty_groups.insert(group.to_owned());
        }
    }
    fn submission(
        &self,
        action_id: &str,
        name: String,
        flavor: String,
        appearance: Option<CharacterAppearanceDraft>,
        limits: CharacterLimits,
    ) -> Result<CharacterSubmission, CharacterValidationError> {
        if self.disposed {
            return Err(CharacterValidationError::Disposed);
        }
        let action = self
            .view
            .actions
            .iter()
            .find(|action| action.id == action_id)
            .ok_or(CharacterValidationError::UnknownSelection)?;
        if !action.enabled || self.view.status == CharacterStatus::Pending {
            return Err(CharacterValidationError::Unavailable);
        }
        for text in [&name, &flavor] {
            validate_text(text, limits.max_text_bytes, true)?;
            if text.encode_utf16().count() > crate::MAX_DRAFT_UTF16_UNITS {
                return Err(CharacterValidationError::ResourceLimit);
            }
        }
        let appearance = if self.view.appearance.is_some() {
            let appearance = appearance.ok_or(CharacterValidationError::Unavailable)?;
            for text in [&appearance.features, &appearance.outfit] {
                validate_text(text, limits.max_text_bytes, true)?;
                if text.encode_utf16().count() > crate::MAX_DRAFT_UTF16_UNITS {
                    return Err(CharacterValidationError::ResourceLimit);
                }
            }
            Some(appearance)
        } else {
            None
        };
        Ok(CharacterSubmission {
            generation: self.view.generation,
            owner_key: self.view.owner_key.clone(),
            revision: self.view.revision,
            action_id: action.id.clone(),
            kind: action.kind,
            choices: self
                .choices
                .iter()
                .map(|(group_id, option_id)| CharacterChoice {
                    group_id: group_id.clone(),
                    option_id: option_id.clone(),
                })
                .collect(),
            name,
            flavor,
            appearance,
        })
    }
    fn dispose(&mut self) {
        self.disposed = true;
        self.choices.clear();
        self.dirty_groups.clear();
        self.view.name.clear();
        self.view.flavor.clear();
        if let Some(appearance) = self.view.appearance.as_mut() {
            appearance.features.clear();
            appearance.outfit.clear();
        }
        self.view.appearance = None;
        self.view.groups.clear();
        self.view.actions.clear();
        self.view.facts.clear();
        self.view.owner_key.clear();
        self.view.chapter.clear();
        self.view.title.clear();
        self.view.description.clear();
        self.view.connection.clear();
        self.view.status_message.clear();
    }
}

fn initial_choice(group: &CharacterGroup) -> Option<(String, String)> {
    group
        .selected
        .as_ref()
        .or_else(|| (group.options.len() == 1).then(|| &group.options[0].id))
        .map(|option| (group.id.clone(), option.clone()))
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
    use web_sys::{Document, Element, Event, EventTarget, HtmlElement, HtmlInputElement, Node};

    #[derive(Debug)]
    pub enum CharacterPhaseError {
        InvalidView(CharacterValidationError),
        Control(ControlError),
        Dom(UiError),
        UpdateCleanup {
            update: Box<CharacterPhaseError>,
            cleanup: Box<CharacterPhaseError>,
        },
    }
    impl fmt::Display for CharacterPhaseError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidView(error) => fmt::Display::fmt(error, formatter),
                Self::Control(error) => fmt::Display::fmt(error, formatter),
                Self::Dom(error) => fmt::Display::fmt(error, formatter),
                Self::UpdateCleanup { update, cleanup } => {
                    write!(formatter, "{update}; cleanup: {cleanup}")
                }
            }
        }
    }
    impl std::error::Error for CharacterPhaseError {}
    impl From<CharacterValidationError> for CharacterPhaseError {
        fn from(error: CharacterValidationError) -> Self {
            Self::InvalidView(error)
        }
    }
    impl From<ControlError> for CharacterPhaseError {
        fn from(error: ControlError) -> Self {
            Self::Control(error)
        }
    }
    impl From<wasm_bindgen::JsValue> for CharacterPhaseError {
        fn from(error: wasm_bindgen::JsValue) -> Self {
            Self::Dom(UiError::Browser(error))
        }
    }

    struct OptionNodes {
        root: Element,
        art: PortraitSlot,
        description: Element,
        availability: Element,
        action: ControlledAction,
    }
    struct GroupNodes {
        root: Element,
        heading: Element,
        description: Element,
        options_root: Element,
        options: BTreeMap<String, OptionNodes>,
    }
    struct AppearanceInputs {
        root: Element,
        features: Rc<ControlledTextInput>,
        outfit: Rc<ControlledTextInput>,
    }
    type SubmissionCallback = Box<dyn FnMut(CharacterSubmission)>;

    struct PortraitListener {
        target: EventTarget,
        event: &'static str,
        active: Rc<Cell<bool>>,
        callback: Option<Closure<dyn FnMut(Event)>>,
    }
    impl PortraitListener {
        fn bind(
            target: EventTarget,
            event: &'static str,
            mut operation: impl FnMut() + 'static,
        ) -> Result<Self, CharacterPhaseError> {
            let active = Rc::new(Cell::new(true));
            let fence = Rc::clone(&active);
            let callback = Closure::wrap(Box::new(move |_| {
                if fence.get() {
                    operation();
                }
            }) as Box<dyn FnMut(Event)>);
            target.add_event_listener_with_callback(event, callback.as_ref().unchecked_ref())?;
            Ok(Self {
                target,
                event,
                active,
                callback: Some(callback),
            })
        }
        fn dispose(&mut self) -> Result<(), CharacterPhaseError> {
            self.active.set(false);
            if let Some(callback) = &self.callback {
                self.target.remove_event_listener_with_callback(
                    self.event,
                    callback.as_ref().unchecked_ref(),
                )?;
            }
            self.callback = None;
            Ok(())
        }
    }
    impl Drop for PortraitListener {
        fn drop(&mut self) {
            // A browser removal failure may retain only a fenced, inert closure.
            if self.dispose().is_err()
                && let Some(callback) = self.callback.take()
            {
                callback.forget();
            }
        }
    }
    struct MountedPortrait {
        image: HtmlElement,
        listeners: Vec<PortraitListener>,
    }
    /// A stable frame with a flat silhouette while optional artwork is pending or
    /// failed. Each source owns a distinct image and two listeners; replacement
    /// fences/removes the old listeners before the new resource can publish.
    pub(crate) struct PortraitSlot {
        frame: HtmlElement,
        fallback: HtmlElement,
        current: RefCell<Option<MountedPortrait>>,
        requested: Cell<Option<CharacterPortrait>>,
        failed: Rc<Cell<bool>>,
    }
    impl PortraitSlot {
        pub(crate) fn create(
            document: &Document,
            parent: &Element,
            class: &str,
        ) -> Result<Self, CharacterPhaseError> {
            let frame = child(document, parent, "div", class, None)?
                .dyn_into::<HtmlElement>()
                .map_err(|_| CharacterPhaseError::Dom(UiError::WrongElementType))?;
            frame.set_attribute("role", "img")?;
            frame.set_attribute(
                "aria-label",
                "Illustrative portrait · optional artwork with flat fallback",
            )?;
            let fallback = child(document, frame.as_ref(), "span", "portrait-fallback", None)?
                .dyn_into::<HtmlElement>()
                .map_err(|_| CharacterPhaseError::Dom(UiError::WrongElementType))?;
            fallback.set_attribute("aria-hidden", "true")?;
            frame.set_hidden(true);
            Ok(Self {
                frame,
                fallback,
                current: RefCell::new(None),
                requested: Cell::new(None),
                failed: Rc::new(Cell::new(false)),
            })
        }
        pub(crate) fn update(
            &self,
            document: &Document,
            art: Option<CharacterPortrait>,
            reset: bool,
        ) -> Result<(), CharacterPhaseError> {
            if !reset && art == self.requested.get() {
                return Ok(());
            }
            self.dispose_image()?;
            self.requested.set(art);
            self.failed.set(false);
            self.fallback.set_hidden(false);
            self.frame.set_hidden(art.is_none());
            let Some(art) = art else {
                return Ok(());
            };
            let image = document
                .create_element("img")?
                .dyn_into::<HtmlElement>()
                .map_err(|_| CharacterPhaseError::Dom(UiError::WrongElementType))?;
            image.set_class_name("portrait-bitmap");
            image.set_hidden(true);
            image.set_attribute("alt", "")?;
            image.set_attribute("aria-hidden", "true")?;
            let mut listeners = Vec::with_capacity(2);
            for event in ["load", "error"] {
                let bitmap = image.clone();
                let fallback = self.fallback.clone();
                let failed = Rc::clone(&self.failed);
                listeners.push(PortraitListener::bind(
                    image.clone().into(),
                    event,
                    move || {
                        let unavailable = event == "error";
                        failed.set(unavailable);
                        bitmap.set_hidden(unavailable);
                        fallback.set_hidden(!unavailable);
                    },
                )?);
            }
            image.set_attribute("src", art.asset_path())?;
            self.frame.append_child(&image)?;
            *self.current.borrow_mut() = Some(MountedPortrait { image, listeners });
            Ok(())
        }
        pub(crate) fn retry(&self, document: &Document) -> Result<(), CharacterPhaseError> {
            if self.failed.get() {
                self.update(document, self.requested.get(), true)?;
            }
            Ok(())
        }
        fn dispose_image(&self) -> Result<(), CharacterPhaseError> {
            let mut failure = None;
            if let Some(mut mounted) = self.current.borrow_mut().take() {
                for listener in &mut mounted.listeners {
                    if let Err(error) = listener.dispose()
                        && failure.is_none()
                    {
                        failure = Some(error);
                    }
                }
                mounted.image.set_hidden(true);
                mounted.image.remove();
            }
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
        pub(crate) fn dispose(&self) -> Result<(), CharacterPhaseError> {
            self.requested.set(None);
            self.failed.set(false);
            self.frame.set_hidden(true);
            self.fallback.set_hidden(true);
            self.dispose_image()
        }
    }
    impl Drop for PortraitSlot {
        fn drop(&mut self) {
            // Listener Drop preserves the inert-closure fallback on DOM failures.
            if self.dispose().is_err() {
                self.frame.remove();
            }
        }
    }

    // Clear existing Text nodes before replacing/detaching them. Removing descendants
    // alone leaves their data readable through independently retained DOM handles.
    fn scrub_text(node: &Node) {
        let mut child = node.first_child();
        while let Some(descendant) = child {
            child = descendant.next_sibling();
            scrub_text(&descendant);
        }
        node.set_node_value(Some(""));
    }

    pub(crate) fn replace_text(node: &Element, text: Option<&str>) {
        scrub_text(node.as_ref());
        node.set_text_content(text);
    }

    pub(crate) fn scrub_private_dom(node: &Node) {
        node.set_node_value(Some(""));
        if let Some(input) = node.dyn_ref::<HtmlInputElement>() {
            input.set_value("");
            input.set_read_only(true);
        }
        let mut child = node.first_child();
        while let Some(descendant) = child {
            child = descendant.next_sibling();
            scrub_private_dom(&descendant);
        }
    }

    /// Mounted Rust character phase with bounded keyed controls. Native buttons
    /// provide keyboard/touch activation; all listeners belong to controlled widgets.
    /// Updates preserve draft fields and keyed button focus within a scope. A scope
    /// change replaces field values and clears obsolete selections before callbacks.
    /// Any DOM/control reconciliation failure terminates the surface and scrubs its
    /// private state. The caller must mount a fresh surface after such a failure.
    pub struct CharacterPhaseSurface {
        document: Document,
        root: Element,
        chapter: Element,
        title: Element,
        description: Element,
        connection: Element,
        status: Element,
        message: Element,
        portrait: PortraitSlot,
        groups_root: Element,
        facts_root: Element,
        actions_root: Element,
        name: Rc<ControlledTextInput>,
        flavor: Rc<ControlledTextInput>,
        workshop: Element,
        identifier: String,
        appearance: Rc<RefCell<Option<AppearanceInputs>>>,
        groups: RefCell<BTreeMap<String, GroupNodes>>,
        facts: RefCell<Vec<(Element, Element, Element)>>,
        actions: RefCell<BTreeMap<String, ControlledAction>>,
        state: Rc<RefCell<CharacterDraftState>>,
        callback: Rc<RefCell<Option<SubmissionCallback>>>,
        limits: CharacterLimits,
    }

    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
        text: Option<&str>,
    ) -> Result<Element, CharacterPhaseError> {
        let node = document.create_element(tag)?;
        node.set_class_name(class);
        node.set_text_content(text);
        parent.append_child(&node)?;
        Ok(node)
    }

    pub(crate) fn place(
        parent: &Element,
        node: &Element,
        cursor: &mut Option<Element>,
    ) -> Result<(), CharacterPhaseError> {
        if cursor
            .as_ref()
            .is_some_and(|next| next.is_same_node(Some(node)))
        {
            *cursor = node.next_element_sibling();
        } else {
            parent.insert_before(node, cursor.as_ref().map(|next| next.as_ref()))?;
        }
        Ok(())
    }

    /// Hides every option whose required group does not hold one of its listed options, and
    /// forgets the player's selection of a hidden option.
    fn refresh_requirements(
        root: &Element,
        state: &Rc<RefCell<CharacterDraftState>>,
    ) -> Result<(), wasm_bindgen::JsValue> {
        let cards = root.query_selector_all("[data-requires-group]")?;
        for index in 0..cards.length() {
            let Some(card) = cards
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let required = card
                .get_attribute("data-requires-group")
                .unwrap_or_default();
            let listed_with = card
                .get_attribute("data-requires-options")
                .unwrap_or_default();
            let chosen = state.borrow().choices.get(&required).cloned();
            if chosen.is_some_and(|chosen| listed_with.split(',').any(|id| id == chosen)) {
                card.remove_attribute("hidden")?;
                continue;
            }
            card.set_attribute("hidden", "")?;
            let own = card
                .query_selector("button")?
                .and_then(|button| button.get_attribute("data-option-id"));
            let group = card
                .closest("[data-group-id]")?
                .and_then(|group| group.get_attribute("data-group-id"));
            if let (Some(own), Some(group)) = (own, group) {
                let mut draft = state.borrow_mut();
                if draft.choices.get(&group) == Some(&own) {
                    draft.choices.remove(&group);
                    draft.dirty_groups.remove(&group);
                }
            }
            if let Some(button) = card.query_selector("button")? {
                button.set_attribute("aria-pressed", "false")?;
            }
            card.set_attribute("data-selected", "false")?;
        }
        let groups = root.query_selector_all("[data-group-id]")?;
        for index in 0..groups.length() {
            let Some(group) = groups
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let Some(group_id) = group.get_attribute("data-group-id") else {
                continue;
            };
            let options =
                group.query_selector_all(":scope > .character-options > .character-option")?;
            let mut visible_options = Vec::new();
            for option_index in 0..options.length() {
                let Some(card) = options
                    .item(option_index)
                    .and_then(|node| node.dyn_into::<Element>().ok())
                else {
                    continue;
                };
                if !card.has_attribute("hidden")
                    && let Some(button) = card.query_selector("button")?
                    && let Some(option_id) = button.get_attribute("data-option-id")
                {
                    visible_options.push(option_id);
                }
            }
            state
                .borrow_mut()
                .select_only_visible_option(&group_id, &visible_options);
            let selected = state.borrow().choices.get(&group_id).cloned();
            for option_index in 0..options.length() {
                let Some(card) = options
                    .item(option_index)
                    .and_then(|node| node.dyn_into::<Element>().ok())
                else {
                    continue;
                };
                if let Some(button) = card.query_selector("button")?
                    && let Some(option_id) = button.get_attribute("data-option-id")
                {
                    let is_selected = selected.as_deref() == Some(&option_id);
                    button.set_attribute(
                        "aria-pressed",
                        if is_selected { "true" } else { "false" },
                    )?;
                    card.set_attribute(
                        "data-selected",
                        if is_selected { "true" } else { "false" },
                    )?;
                }
            }
        }
        Ok(())
    }

    fn input_view<'a>(label: &'a str, view: &'a CharacterPhaseView) -> TextInputView<'a> {
        TextInputView {
            label,
            enabled: view.editable
                && matches!(
                    view.status,
                    CharacterStatus::Editing | CharacterStatus::Rejected
                ),
            feedback: match view.status {
                CharacterStatus::Pending => InputFeedback::Pending(&view.status_message),
                CharacterStatus::Rejected => InputFeedback::Rejected(&view.status_message),
                _ => InputFeedback::None,
            },
        }
    }

    impl CharacterPhaseSurface {
        /// Identifier follows the ControlledTextInput contract and must be unique.
        /// Callback emits input only: its invocation never changes reported readiness.
        pub fn create(
            document: &Document,
            identifier: &str,
            view: &CharacterPhaseView,
            limits: CharacterLimits,
            on_submit: impl FnMut(CharacterSubmission) + 'static,
        ) -> Result<Self, CharacterPhaseError> {
            view.validate(limits)?;
            let root = document.create_element("section")?;
            root.set_class_name("df-character");
            child(
                document,
                &root,
                "style",
                "",
                Some(crate::character_phase_theme::STYLES),
            )?;
            let backdrop = child(document, &root, "img", "character-backdrop", None)?;
            backdrop.set_attribute("src", "assets/concept-art/scene-campfire-under-stars.webp")?;
            backdrop.set_attribute("alt", "")?;
            let header = child(document, &root, "header", "character-header", None)?;
            let brand = child(document, &header, "div", "character-brand", Some("Dungeon"))?;
            child(document, &brand, "span", "", Some("Flux"))?;
            child(
                document,
                &header,
                "span",
                "character-overline",
                Some("A story begins with you"),
            )?;
            let hero = child(document, &root, "div", "character-hero", None)?;
            let chapter = child(document, &hero, "p", "character-overline", None)?;
            let title = child(document, &hero, "h1", "", None)?;
            let description = child(document, &hero, "p", "character-description", None)?;
            let layout = child(document, &root, "div", "character-layout", None)?;
            let workshop = child(document, &layout, "div", "character-workshop", None)?;
            let identity = child(
                document,
                &workshop,
                "section",
                "character-panel identity-panel",
                None,
            )?;
            child(document, &identity, "h2", "", Some("Name your hero"))?;
            let name = Rc::new(ControlledTextInput::create(
                document,
                &format!("{identifier}-name"),
                input_view("Character name", view),
                &view.name,
            )?);
            name.input().set_attribute("autocomplete", "off")?;
            let flavor = Rc::new(ControlledTextInput::create(
                document,
                &format!("{identifier}-flavor"),
                input_view("A detail that makes you memorable", view),
                &view.flavor,
            )?);
            identity.append_child(name.root())?;
            identity.append_child(flavor.root())?;
            let groups_root = child(document, &workshop, "div", "character-groups", None)?;
            let aside = child(
                document,
                &layout,
                "aside",
                "character-panel character-summary",
                None,
            )?;
            child(
                document,
                &aside,
                "p",
                "character-overline",
                Some("How your hero looks"),
            )?;
            let portrait = PortraitSlot::create(document, &aside, "character-portrait")?;
            child(
                document,
                &aside,
                "p",
                "character-art-credit",
                Some("Illustrative concept portraits · includes AI-generated artwork"),
            )?;
            let status = child(document, &aside, "h2", "character-status", None)?;
            let message = child(document, &aside, "p", "character-message", None)?;
            message.set_attribute("role", "status")?;
            message.set_attribute("aria-live", "polite")?;
            message.set_attribute("aria-atomic", "true")?;
            let facts_root = child(document, &aside, "dl", "character-facts", None)?;
            let actions_root = child(document, &aside, "div", "character-actions", None)?;
            let reference = child(document, &root, "details", "character-reference", None)?;
            child(
                document,
                &reference,
                "summary",
                "",
                Some("Original character-creation art direction"),
            )?;
            child(
                document,
                &reference,
                "p",
                "character-art-credit",
                Some(
                    "ShellHacks interface reference · the controls above are mounted Rust components.",
                ),
            )?;
            let image = child(document, &reference, "img", "", None)?;
            image.set_attribute(
                "src",
                "assets/concept-art/ui-tv-character-creation-phone-picker.webp",
            )?;
            image.set_attribute(
                "alt",
                "Original character creation television and phone interface concept",
            )?;
            image.set_attribute("loading", "lazy")?;
            let connection = child(document, &root, "footer", "character-connection", None)?;
            let surface = Self {
                document: document.clone(),
                root,
                chapter,
                title,
                description,
                connection,
                status,
                message,
                portrait,
                groups_root,
                facts_root,
                actions_root,
                name,
                flavor,
                workshop,
                identifier: identifier.to_owned(),
                appearance: Rc::new(RefCell::new(None)),
                groups: RefCell::new(BTreeMap::new()),
                facts: RefCell::new(Vec::new()),
                actions: RefCell::new(BTreeMap::new()),
                state: Rc::new(RefCell::new(CharacterDraftState::create(view))),
                callback: Rc::new(RefCell::new(Some(Box::new(on_submit)))),
                limits,
            };
            surface.update(view)?;
            Ok(surface)
        }
        pub fn root(&self) -> &Element {
            &self.root
        }
        pub fn name_input(&self) -> &ControlledTextInput {
            &self.name
        }
        pub fn flavor_input(&self) -> &ControlledTextInput {
            &self.flavor
        }
        pub fn features_input(&self) -> Option<Rc<ControlledTextInput>> {
            self.appearance
                .borrow()
                .as_ref()
                .map(|inputs| Rc::clone(&inputs.features))
        }
        pub fn outfit_input(&self) -> Option<Rc<ControlledTextInput>> {
            self.appearance
                .borrow()
                .as_ref()
                .map(|inputs| Rc::clone(&inputs.outfit))
        }

        /// Explicit presentation retry for currently failed optional portraits.
        /// Ordinary view updates never retry an unchanged failed source. Each retry
        /// replaces only that image's owned listeners; it submits no game command.
        pub fn retry_failed_portraits(&self) -> Result<(), CharacterPhaseError> {
            if self.state.borrow().disposed {
                return Err(CharacterValidationError::Disposed.into());
            }
            let result = (|| -> Result<(), CharacterPhaseError> {
                self.portrait.retry(&self.document)?;
                for group in self.groups.borrow().values() {
                    for option in group.options.values() {
                        option.art.retry(&self.document)?;
                    }
                }
                Ok(())
            })();
            match result {
                Ok(()) => Ok(()),
                Err(update) => match self.dispose() {
                    Ok(()) => Err(update),
                    Err(cleanup) => Err(CharacterPhaseError::UpdateCleanup {
                        update: Box::new(update),
                        cleanup: Box::new(cleanup),
                    }),
                },
            }
        }

        pub fn update(&self, view: &CharacterPhaseView) -> Result<(), CharacterPhaseError> {
            view.validate(self.limits)?;
            // Freshness checks do not mutate state. Any fallible DOM reconciliation
            // failure terminates the scope rather than retaining mixed ownership.
            match self.update_view(view) {
                Ok(()) => Ok(()),
                Err(CharacterPhaseError::InvalidView(error)) => Err(error.into()),
                Err(update) => match self.dispose() {
                    Ok(()) => Err(update),
                    Err(cleanup) => Err(CharacterPhaseError::UpdateCleanup {
                        update: Box::new(update),
                        cleanup: Box::new(cleanup),
                    }),
                },
            }
        }

        fn update_view(&self, view: &CharacterPhaseView) -> Result<(), CharacterPhaseError> {
            let focused = self
                .document
                .active_element()
                .filter(|node| self.root.contains(Some(node)));
            let replaced = self.state.borrow_mut().reconcile(view)?;
            scrub_text(self.name.root().as_ref());
            scrub_text(self.flavor.root().as_ref());
            self.name.update(
                input_view("Character name", view),
                if replaced
                    || matches!(
                        view.status,
                        CharacterStatus::Ready | CharacterStatus::Locked
                    )
                {
                    DraftUpdate::Replace(&view.name)
                } else {
                    DraftUpdate::Preserve
                },
            )?;
            self.flavor.update(
                input_view("A detail that makes you memorable", view),
                if replaced
                    || matches!(
                        view.status,
                        CharacterStatus::Ready | CharacterStatus::Locked
                    )
                {
                    DraftUpdate::Replace(&view.flavor)
                } else {
                    DraftUpdate::Preserve
                },
            )?;
            self.update_appearance(view, replaced)?;
            for (node, text) in [
                (&self.chapter, view.chapter.as_str()),
                (&self.title, &view.title),
                (&self.description, &view.description),
                (&self.connection, &view.connection),
                (&self.status, view.status.label()),
                (&self.message, &view.status_message),
            ] {
                replace_text(node, Some(text));
            }
            self.root.set_attribute(
                "data-status",
                match view.status {
                    CharacterStatus::Editing => "editing",
                    CharacterStatus::Pending => "pending",
                    CharacterStatus::Rejected => "rejected",
                    CharacterStatus::Ready => "ready",
                    CharacterStatus::Locked => "locked",
                },
            )?;
            self.root
                .set_attribute("data-generation", &view.generation.to_string())?;
            self.portrait
                .update(&self.document, view.portrait, replaced)?;
            self.update_groups(view, replaced)?;
            refresh_requirements(&self.root, &self.state)?;
            self.update_facts(view)?;
            self.update_actions(view)?;
            if let Some(node) = focused
                && node.is_connected()
                && !self
                    .document
                    .active_element()
                    .as_ref()
                    .is_some_and(|active| node.is_same_node(Some(active)))
                && let Some(element) = node.dyn_ref::<HtmlElement>()
            {
                element.focus()?;
            }
            Ok(())
        }

        fn update_appearance(
            &self,
            view: &CharacterPhaseView,
            replaced: bool,
        ) -> Result<(), CharacterPhaseError> {
            let mut mounted = self.appearance.borrow_mut();
            if (replaced || view.appearance.is_none())
                && let Some(inputs) = mounted.take()
            {
                scrub_private_dom(inputs.root.as_ref());
                inputs.features.dispose()?;
                inputs.outfit.dispose()?;
                inputs.root.remove();
            }
            let Some(appearance) = &view.appearance else {
                return Ok(());
            };
            if mounted.is_none() {
                let root = child(
                    &self.document,
                    &self.workshop,
                    "section",
                    "character-panel character-appearance",
                    None,
                )?;
                child(
                    &self.document,
                    &root,
                    "p",
                    "character-overline",
                    Some("Appearance"),
                )?;
                child(&self.document, &root, "h2", "", Some("How do you look?"))?;
                child(
                    &self.document,
                    &root,
                    "p",
                    "character-group-description",
                    Some(
                        "Describe visual features and clothing for your character reference sheet. These details do not change abilities or equipment.",
                    ),
                )?;
                let features = Rc::new(ControlledTextInput::create(
                    &self.document,
                    &format!("{}-features", self.identifier),
                    input_view("Visual features", view),
                    &appearance.features,
                )?);
                features.input().set_attribute("autocomplete", "off")?;
                let outfit = Rc::new(ControlledTextInput::create(
                    &self.document,
                    &format!("{}-outfit", self.identifier),
                    input_view("Clothing and outfit", view),
                    &appearance.outfit,
                )?);
                outfit.input().set_attribute("autocomplete", "off")?;
                root.append_child(features.root())?;
                root.append_child(outfit.root())?;
                child(
                    &self.document,
                    &root,
                    "p",
                    "character-appearance-note",
                    Some(
                        "After your character is created, a saved reference sheet is generated in the background and reused in future scenes.",
                    ),
                )?;
                self.workshop
                    .insert_before(&root, Some(&self.groups_root))?;
                *mounted = Some(AppearanceInputs {
                    root,
                    features,
                    outfit,
                });
                return Ok(());
            }
            if let Some(inputs) = mounted.as_ref() {
                for (input, value, label) in [
                    (
                        &inputs.features,
                        appearance.features.as_str(),
                        "Visual features",
                    ),
                    (
                        &inputs.outfit,
                        appearance.outfit.as_str(),
                        "Clothing and outfit",
                    ),
                ] {
                    scrub_text(input.root().as_ref());
                    input.update(
                        input_view(label, view),
                        if replaced
                            || matches!(
                                view.status,
                                CharacterStatus::Ready | CharacterStatus::Locked
                            )
                        {
                            DraftUpdate::Replace(value)
                        } else {
                            DraftUpdate::Preserve
                        },
                    )?;
                }
            }
            Ok(())
        }

        fn update_groups(
            &self,
            view: &CharacterPhaseView,
            replaced: bool,
        ) -> Result<(), CharacterPhaseError> {
            let mut groups = self.groups.borrow_mut();
            let obsolete: Vec<_> = groups
                .keys()
                .filter(|key| !view.groups.iter().any(|group| group.id == **key))
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(group) = groups.remove(&key) {
                    scrub_private_dom(group.root.as_ref());
                    for option in group.options.values() {
                        option.art.dispose()?;
                        option.action.dispose()?;
                    }
                    self.groups_root.remove_child(&group.root)?;
                }
            }
            let mut group_cursor = self.groups_root.first_element_child();
            for group in &view.groups {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    groups.entry(group.id.clone())
                {
                    let root = self.document.create_element("section")?;
                    root.set_class_name("character-panel character-choice-group");
                    root.set_attribute("data-group-id", &group.id)?;
                    let heading = child(&self.document, &root, "h2", "", None)?;
                    let description = child(
                        &self.document,
                        &root,
                        "p",
                        "character-group-description",
                        None,
                    )?;
                    let options_root =
                        child(&self.document, &root, "div", "character-options", None)?;
                    entry.insert(GroupNodes {
                        root,
                        heading,
                        description,
                        options_root,
                        options: BTreeMap::new(),
                    });
                }
                let Some(nodes) = groups.get_mut(&group.id) else {
                    continue;
                };
                replace_text(&nodes.heading, Some(&group.label));
                replace_text(&nodes.description, Some(&group.description));
                place(&self.groups_root, &nodes.root, &mut group_cursor)?;
                nodes.root.set_attribute(
                    "data-choice-group",
                    if group.options.len() == 1 {
                        "false"
                    } else {
                        "true"
                    },
                )?;
                let obsolete: Vec<_> = nodes
                    .options
                    .keys()
                    .filter(|key| !group.options.iter().any(|option| option.id == **key))
                    .cloned()
                    .collect();
                for key in obsolete {
                    if let Some(option) = nodes.options.remove(&key) {
                        scrub_private_dom(option.root.as_ref());
                        option.art.dispose()?;
                        option.action.dispose()?;
                        nodes.options_root.remove_child(&option.root)?;
                    }
                }
                let mut cursor = nodes.options_root.first_element_child();
                for option in &group.options {
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        nodes.options.entry(option.id.clone())
                    {
                        let root = self.document.create_element("article")?;
                        root.set_class_name("character-option");
                        let art = PortraitSlot::create(&self.document, &root, "option-portrait")?;
                        let description =
                            child(&self.document, &root, "p", "option-description", None)?;
                        let availability =
                            child(&self.document, &root, "p", "option-availability", None)?;
                        let action = ControlledAction::create(
                            &self.document,
                            ActionView {
                                label: &option.label,
                                enabled: option.enabled,
                                pending: false,
                            },
                        )?;
                        action
                            .element()
                            .set_attribute("data-option-id", &option.id)?;
                        root.append_child(action.element())?;
                        let state = Rc::clone(&self.state);
                        let group_id = group.id.clone();
                        let option_id = option.id.clone();
                        let container = nodes.options_root.clone();
                        let surface_root = self.root.clone();
                        let message = self.message.clone();
                        action.on_activate(move || {
                            if state.borrow_mut().choose(&group_id, &option_id).is_err() {
                                replace_text(&message, Some("This offered choice is no longer available. Await a current view."));
                                return;
                            }
                            let result = (|| -> Result<(), wasm_bindgen::JsValue> {
                                let mut card = container.first_element_child();
                                while let Some(node) = card {
                                    if let Some(button) = node.query_selector("button")? {
                                        let selected = button.get_attribute("data-option-id").as_deref() == Some(&option_id);
                                        button.set_attribute("aria-pressed", if selected { "true" } else { "false" })?;
                                        node.set_attribute("data-selected", if selected { "true" } else { "false" })?;
                                    }
                                    card = node.next_element_sibling();
                                }
                                Ok(())
                            })();
                            if result.is_err() || refresh_requirements(&surface_root, &state).is_err() { replace_text(&message, Some("Choice recorded locally; its presentation could not refresh.")); }
                        })?;
                        entry.insert(OptionNodes {
                            root,
                            art,
                            description,
                            availability,
                            action,
                        });
                    }
                    if let Some(card) = nodes.options.get(&option.id) {
                        scrub_text(card.action.element().as_ref());
                        let fact = group.options.len() == 1;
                        card.action.update(ActionView {
                            label: &option.label,
                            enabled: self.state.borrow().editable() && option.enabled && !fact,
                            pending: view.status == CharacterStatus::Pending,
                        })?;
                        let selected =
                            self.state.borrow().choices.get(&group.id) == Some(&option.id);
                        card.action.element().set_attribute(
                            "aria-pressed",
                            if selected { "true" } else { "false" },
                        )?;
                        card.root.set_attribute(
                            "data-selected",
                            if selected { "true" } else { "false" },
                        )?;
                        card.root.set_attribute(
                            "data-choice-option",
                            if fact { "false" } else { "true" },
                        )?;
                        replace_text(&card.description, Some(&option.description));
                        replace_text(&card.availability, Some(&option.availability));
                        card.art.update(&self.document, option.portrait, replaced)?;
                        place(&nodes.options_root, &card.root, &mut cursor)?;
                    }
                }
            }
            Ok(())
        }

        /// Lists `option` of `group` only while `requires_group` holds one of `requires_options`
        /// (the server says which: an appearance goes with the packages that grant its weapon).
        /// An option that stops being listed loses the player's selection of it.
        pub fn require_option(
            &self,
            group: &str,
            option: &str,
            requires_group: &str,
            requires_options: &[String],
        ) -> Result<(), CharacterPhaseError> {
            if let Some(card) = self
                .groups
                .borrow()
                .get(group)
                .and_then(|nodes| nodes.options.get(option))
            {
                card.root
                    .set_attribute("data-requires-group", requires_group)?;
                card.root
                    .set_attribute("data-requires-options", &requires_options.join(","))?;
            }
            refresh_requirements(&self.root, &self.state)?;
            Ok(())
        }

        fn update_facts(&self, view: &CharacterPhaseView) -> Result<(), CharacterPhaseError> {
            let mut facts = self.facts.borrow_mut();
            while facts.len() > view.facts.len() {
                if let Some((root, _, _)) = facts.pop() {
                    scrub_private_dom(root.as_ref());
                    self.facts_root.remove_child(&root)?;
                }
            }
            while facts.len() < view.facts.len() {
                let root = child(
                    &self.document,
                    &self.facts_root,
                    "div",
                    "character-fact",
                    None,
                )?;
                let label = child(&self.document, &root, "dt", "", None)?;
                let value = child(&self.document, &root, "dd", "", None)?;
                facts.push((root, label, value));
            }
            for ((_, label, value), fact) in facts.iter().zip(&view.facts) {
                replace_text(label, Some(&fact.label));
                replace_text(value, Some(&fact.value));
            }
            Ok(())
        }

        fn update_actions(&self, view: &CharacterPhaseView) -> Result<(), CharacterPhaseError> {
            let mut actions = self.actions.borrow_mut();
            let obsolete: Vec<_> = actions
                .keys()
                .filter(|key| !view.actions.iter().any(|action| action.id == **key))
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(action) = actions.remove(&key) {
                    scrub_private_dom(action.element().as_ref());
                    action.dispose()?;
                }
            }
            let mut cursor = self.actions_root.first_element_child();
            for offered in &view.actions {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    actions.entry(offered.id.clone())
                {
                    let action = ControlledAction::create(
                        &self.document,
                        ActionView {
                            label: &offered.label,
                            enabled: offered.enabled,
                            pending: view.status == CharacterStatus::Pending,
                        },
                    )?;
                    action
                        .element()
                        .set_attribute("data-action-id", &offered.id)?;
                    let state = Rc::clone(&self.state);
                    let callback = Rc::clone(&self.callback);
                    let name = Rc::clone(&self.name);
                    let flavor = Rc::clone(&self.flavor);
                    let appearance = Rc::clone(&self.appearance);
                    let action_id = offered.id.clone();
                    let limits = self.limits;
                    let message = self.message.clone();
                    action.on_activate(move || {
                        let submission = state.borrow().submission(
                            &action_id,
                            name.draft(),
                            flavor.draft(),
                            appearance
                                .borrow()
                                .as_ref()
                                .map(|inputs| CharacterAppearanceDraft {
                                    features: inputs.features.draft(),
                                    outfit: inputs.outfit.draft(),
                                }),
                            limits,
                        );
                        match submission {
                            Ok(submission) => {
                                let owned = callback.borrow_mut().take();
                                if let Some(mut operation) = owned {
                                    operation(submission);
                                    if !state.borrow().disposed {
                                        *callback.borrow_mut() = Some(operation);
                                    }
                                }
                            }
                            Err(error) => replace_text(&message, Some(&error.to_string())),
                        }
                    })?;
                    entry.insert(action);
                }
                if let Some(action) = actions.get(&offered.id) {
                    scrub_text(action.element().as_ref());
                    action.update(ActionView {
                        label: &offered.label,
                        enabled: offered.enabled,
                        pending: view.status == CharacterStatus::Pending,
                    })?;
                    place(&self.actions_root, action.element().as_ref(), &mut cursor)?;
                }
            }
            Ok(())
        }

        /// Idempotent terminal teardown fences callbacks, scrubs retained DOM text
        /// and private state, then attempts every owned removal even after a failure.
        pub fn dispose(&self) -> Result<(), CharacterPhaseError> {
            self.state.borrow_mut().dispose();
            self.callback.borrow_mut().take();
            scrub_private_dom(self.root.as_ref());
            for node in [
                &self.chapter,
                &self.title,
                &self.description,
                &self.connection,
                &self.status,
                &self.message,
                self.name.root(),
                self.flavor.root(),
            ] {
                scrub_private_dom(node.as_ref());
            }
            let mut appearance_failure = None;
            if let Some(inputs) = self.appearance.borrow_mut().take() {
                scrub_private_dom(inputs.root.as_ref());
                if let Err(error) = inputs.features.dispose() {
                    appearance_failure = Some(CharacterPhaseError::from(error));
                }
                if let Err(error) = inputs.outfit.dispose() {
                    appearance_failure.get_or_insert_with(|| CharacterPhaseError::from(error));
                }
                inputs.root.remove();
            }
            let mut failure = appearance_failure;
            let mut record = |result: Result<(), CharacterPhaseError>| {
                if let Err(error) = result
                    && failure.is_none()
                {
                    failure = Some(error);
                }
            };
            record(self.portrait.dispose());
            record(self.name.dispose().map_err(Into::into));
            record(self.flavor.dispose().map_err(Into::into));
            for group in self.groups.borrow().values() {
                scrub_private_dom(group.root.as_ref());
                scrub_private_dom(group.heading.as_ref());
                scrub_private_dom(group.description.as_ref());
                for option in group.options.values() {
                    scrub_private_dom(option.root.as_ref());
                    scrub_private_dom(option.description.as_ref());
                    scrub_private_dom(option.availability.as_ref());
                    scrub_private_dom(option.action.element().as_ref());
                    record(option.art.dispose());
                    record(option.action.dispose().map_err(Into::into));
                    record(
                        option
                            .action
                            .element()
                            .remove_attribute("data-option-id")
                            .map_err(Into::into),
                    );
                }
            }
            for (_, label, value) in self.facts.borrow().iter() {
                scrub_private_dom(label.as_ref());
                scrub_private_dom(value.as_ref());
            }
            for action in self.actions.borrow().values() {
                scrub_private_dom(action.element().as_ref());
                record(action.dispose().map_err(Into::into));
                record(
                    action
                        .element()
                        .remove_attribute("data-action-id")
                        .map_err(Into::into),
                );
            }
            self.groups.borrow_mut().clear();
            self.actions.borrow_mut().clear();
            self.facts.borrow_mut().clear();
            if let Some(parent) = self.root.parent_node() {
                record(
                    parent
                        .remove_child(&self.root)
                        .map(|_| ())
                        .map_err(Into::into),
                );
            }
            self.root.set_text_content(None);
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }
    impl Drop for CharacterPhaseSurface {
        fn drop(&mut self) {
            // Drop cannot return cleanup errors. Explicit disposal reports them;
            // its complete cleanup still runs here, with a final detach fallback.
            if self.dispose().is_err() {
                self.root.remove();
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{CharacterPhaseError, CharacterPhaseSurface};
#[cfg(target_arch = "wasm32")]
pub(crate) use browser::{PortraitSlot, place, replace_text, scrub_private_dom};

#[cfg(test)]
mod tests {
    use super::*;
    fn limits() -> CharacterLimits {
        CharacterLimits {
            max_groups: 8,
            max_options: 64,
            max_facts: 64,
            max_actions: 8,
            max_text_bytes: 4096,
        }
    }
    fn view() -> CharacterPhaseView {
        CharacterPhaseView {
            generation: 1,
            owner_key: "synthetic-owner".into(),
            revision: 1,
            chapter: "Preview".into(),
            title: "Character".into(),
            description: "Synthetic".into(),
            connection: "No session".into(),
            status: CharacterStatus::Editing,
            status_message: "Editing supplied offers".into(),
            editable: true,
            name: "Name".into(),
            flavor: String::new(),
            appearance: None,
            portrait: None,
            groups: vec![CharacterGroup {
                id: "group".into(),
                label: "Choice".into(),
                description: "Offered".into(),
                selected: Some("first".into()),
                options: vec![
                    CharacterOption {
                        id: "first".into(),
                        label: "First".into(),
                        description: "One".into(),
                        availability: "Available".into(),
                        enabled: true,
                        portrait: None,
                    },
                    CharacterOption {
                        id: "second".into(),
                        label: "Second".into(),
                        description: "Two".into(),
                        availability: "Available".into(),
                        enabled: true,
                        portrait: None,
                    },
                ],
            }],
            facts: vec![],
            actions: vec![CharacterAction {
                id: "submit-offer".into(),
                kind: CharacterActionKind::SubmitDraft,
                label: "Send draft".into(),
                enabled: true,
            }],
        }
    }
    #[test]
    fn rejected_and_repeated_updates_preserve_valid_local_selections() {
        let mut view = view();
        let mut state = CharacterDraftState::create(&view);
        state.choose("group", "second").expect("offered choice");
        for revision in 2..66 {
            view.revision = revision;
            state.reconcile(&view).expect("current view");
        }
        view.status = CharacterStatus::Rejected;
        view.revision += 1;
        state.reconcile(&view).expect("rejection");
        let sent = state
            .submission(
                "submit-offer",
                "local name".into(),
                "local flavor".into(),
                None,
                limits(),
            )
            .expect("draft");
        assert_eq!(
            sent.choices,
            vec![CharacterChoice {
                group_id: "group".into(),
                option_id: "second".into()
            }]
        );
        assert_eq!(sent.name, "local name");
        assert_eq!(state.view.status, CharacterStatus::Rejected);
    }
    #[test]
    fn a_single_offered_option_is_submitted_as_a_fact() {
        let mut data = view();
        data.groups[0].selected = None;
        data.groups[0].options.truncate(1);
        let state = CharacterDraftState::create(&data);
        let submission = state
            .submission("submit-offer", "Mara".into(), "".into(), None, limits())
            .expect("single option is included");
        assert_eq!(
            submission.choices,
            vec![CharacterChoice {
                group_id: "group".into(),
                option_id: "first".into(),
            }]
        );
    }
    #[test]
    fn a_conditionally_visible_single_option_is_submitted_and_restored_after_switching_back() {
        let mut data = view();
        data.groups[0].selected = None;
        let mut state = CharacterDraftState::create(&data);
        state.select_only_visible_option("group", &["first".into()]);
        assert_eq!(
            state.choices.get("group").map(String::as_str),
            Some("first")
        );
        assert_eq!(
            state
                .submission("submit-offer", "Mara".into(), "".into(), None, limits())
                .expect("sole visible option is included")
                .choices,
            vec![CharacterChoice {
                group_id: "group".into(),
                option_id: "first".into(),
            }]
        );

        state.select_only_visible_option("group", &["second".into()]);
        assert_eq!(
            state.choices.get("group").map(String::as_str),
            Some("second")
        );
        state.select_only_visible_option("group", &["first".into()]);
        assert_eq!(
            state.choices.get("group").map(String::as_str),
            Some("first")
        );
    }
    #[test]
    fn removed_disabled_and_unknown_offers_cannot_be_selected_or_submitted() {
        let mut view = view();
        let mut state = CharacterDraftState::create(&view);
        assert_eq!(
            state.choose("group", "invented"),
            Err(CharacterValidationError::UnknownSelection)
        );
        state.choose("group", "second").expect("offered");
        view.groups[0].options[1].enabled = false;
        view.revision += 1;
        state.reconcile(&view).expect("revoked choice");
        assert_eq!(
            state.choices.get("group").map(String::as_str),
            Some("first")
        );
        assert_eq!(
            state.choose("group", "second"),
            Err(CharacterValidationError::Unavailable)
        );
        view.actions.clear();
        view.revision += 1;
        state.reconcile(&view).expect("revoked action");
        assert_eq!(
            state.submission("submit-offer", "".into(), "".into(), None, limits()),
            Err(CharacterValidationError::UnknownSelection)
        );
    }
    #[test]
    fn readiness_is_reported_and_generation_or_owner_changes_reset_ownership() {
        let mut view = view();
        let mut state = CharacterDraftState::create(&view);
        state.choose("group", "second").expect("offered");
        state
            .submission("submit-offer", "Name".into(), "".into(), None, limits())
            .expect("input");
        assert_eq!(state.view.status, CharacterStatus::Editing);
        view.status = CharacterStatus::Pending;
        view.revision += 1;
        state.reconcile(&view).expect("pending");
        assert_eq!(
            state.submission("submit-offer", "Name".into(), "".into(), None, limits()),
            Err(CharacterValidationError::Unavailable)
        );
        view.status = CharacterStatus::Locked;
        view.revision += 1;
        state.reconcile(&view).expect("locked");
        assert_eq!(
            state.choices.get("group").map(String::as_str),
            Some("first")
        );
        assert_eq!(
            state.choose("group", "second"),
            Err(CharacterValidationError::Unavailable)
        );
        view.generation = 2;
        view.revision = 0;
        assert_eq!(state.reconcile(&view), Ok(true));
        view.owner_key = "new-owner".into();
        assert_eq!(state.reconcile(&view), Ok(true));
        state.dispose();
        state.dispose();
        assert!(state.choices.is_empty());
        assert!(state.view.name.is_empty());
        assert!(state.view.chapter.is_empty());
        assert!(state.view.title.is_empty());
        assert!(state.view.description.is_empty());
        assert!(state.view.connection.is_empty());
        assert!(state.view.status_message.is_empty());
        assert_eq!(
            state.reconcile(&view),
            Err(CharacterValidationError::Disposed)
        );
    }
    #[test]
    fn bounded_plain_text_duplicate_ids_and_stale_views_are_rejected() {
        let mut data = view();
        assert_eq!(data.validate(limits()), Ok(()));
        data.title = "<img src=x onerror=alert(1)>".into();
        assert_eq!(data.validate(limits()), Ok(())); // Rendered through textContent.
        data.name = "hidden\u{0}control".into();
        assert_eq!(
            data.validate(limits()),
            Err(CharacterValidationError::InvalidText)
        );
        data.name = "😀".repeat(2049);
        assert_eq!(
            data.validate(limits()),
            Err(CharacterValidationError::ResourceLimit)
        );
        data = view();
        data.groups[0].options[1].id = "first".into();
        assert_eq!(
            data.validate(limits()),
            Err(CharacterValidationError::DuplicateId)
        );
        data = view();
        data.groups[0].selected = Some("absent".into());
        assert_eq!(
            data.validate(limits()),
            Err(CharacterValidationError::UnknownSelection)
        );
        let mut state = CharacterDraftState::create(&view());
        let mut stale = view();
        stale.revision = 0;
        assert_eq!(
            state.reconcile(&stale),
            Err(CharacterValidationError::StaleRevision)
        );
        assert_eq!(state.view.revision, 1);
        assert_eq!(
            state.submission("submit-offer", "x\n".into(), "".into(), None, limits()),
            Err(CharacterValidationError::InvalidText)
        );
    }
    #[test]
    fn appearance_is_bounded_optional_and_scoped_to_current_offer() {
        let mut data = view();
        data.appearance = Some(CharacterAppearanceDraft {
            features: "Silver braid".into(),
            outfit: "Blue travel coat".into(),
        });
        assert_eq!(data.validate(limits()), Ok(()));
        let mut state = CharacterDraftState::create(&data);
        let draft = CharacterAppearanceDraft {
            features: "Freckled cheeks".into(),
            outfit: "Green cloak".into(),
        };
        let submitted = state
            .submission(
                "submit-offer",
                "Mara".into(),
                "Story".into(),
                Some(draft.clone()),
                limits(),
            )
            .expect("offered appearance");
        assert_eq!(submitted.appearance, Some(draft.clone()));
        assert_eq!(submitted.revision, 1);
        assert_eq!(state.view.status, CharacterStatus::Editing);
        let mut invalid = data.clone();
        invalid.appearance.as_mut().expect("offered").features = "bad\ntext".into();
        assert_eq!(
            invalid.validate(limits()),
            Err(CharacterValidationError::InvalidText)
        );
        invalid.appearance.as_mut().expect("offered").features = "😀".repeat(2049);
        assert_eq!(
            invalid.validate(limits()),
            Err(CharacterValidationError::ResourceLimit)
        );
        let oversized = CharacterAppearanceDraft {
            features: "x".repeat(4097),
            outfit: String::new(),
        };
        assert_eq!(
            state.submission(
                "submit-offer",
                "Mara".into(),
                "".into(),
                Some(oversized),
                limits()
            ),
            Err(CharacterValidationError::ResourceLimit)
        );
        data.revision += 1;
        data.appearance = None;
        state.reconcile(&data).expect("withdrawn");
        assert_eq!(
            state
                .submission(
                    "submit-offer",
                    "Mara".into(),
                    "".into(),
                    Some(draft),
                    limits()
                )
                .expect("legacy submission")
                .appearance,
            None
        );
        data.revision += 1;
        data.appearance = Some(CharacterAppearanceDraft {
            features: "Confirmed".into(),
            outfit: "Wool cloak".into(),
        });
        data.status = CharacterStatus::Locked;
        state.reconcile(&data).expect("locked view");
        assert_eq!(state.view.appearance, data.appearance);
        data.owner_key = "new-owner".into();
        data.revision = 1;
        state.reconcile(&data).expect("owner replacement");
        assert_eq!(state.view.owner_key, "new-owner");
        state.dispose();
        assert!(state.view.appearance.is_none());
    }
    #[test]
    fn identical_duplicate_is_idempotent_but_equal_revision_conflicts_are_rejected() {
        let original = view();
        let mut state = CharacterDraftState::create(&original);
        state.choose("group", "second").expect("offered choice");
        assert_eq!(state.reconcile(&original), Ok(false));
        let mut conflict = original.clone();
        conflict.status = CharacterStatus::Locked;
        conflict.actions.clear();
        assert_eq!(
            state.reconcile(&conflict),
            Err(CharacterValidationError::StaleRevision)
        );
        assert_eq!(state.view, original);
        assert_eq!(
            state.choices.get("group").map(String::as_str),
            Some("second")
        );
        conflict.revision += 1;
        assert_eq!(state.reconcile(&conflict), Ok(false));
        assert_eq!(state.view.status, CharacterStatus::Locked);
    }
}
