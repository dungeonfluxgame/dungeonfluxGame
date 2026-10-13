use std::{collections::BTreeSet, fmt};

use df_types::{ClientBindingId, SessionRevision};

use crate::{CampaignLimits, CampaignValidationError, CampaignView};

/// Closed portrait allowlist; caller text and identifiers never become asset URLs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorationPortrait {
    Vell,
    Narrator,
    Courier,
    Bandit,
}
impl ExplorationPortrait {
    pub const fn asset_path(self) -> &'static str {
        match self {
            Self::Vell => "assets/concept-art/vell-avatar.webp",
            Self::Narrator => "assets/concept-art/dm-avatar.webp",
            // Generated art is served extensionless; an absent file hides the portrait.
            Self::Courier => "assets/ui/generated/portrait-courier",
            Self::Bandit => "assets/ui/generated/portrait-bandit",
        }
    }
}

pub struct ExplorationNpc<'a> {
    pub name: &'a str,
    pub context: &'a str,
    pub dialogue: &'a str,
    pub portrait: Option<ExplorationPortrait>,
}

/// Opaque offered identifier is returned byte-for-byte. Disabled reasons are
/// supplied by the caller; this component neither derives nor checks game legality.
pub struct ExplorationChoice<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub detail: &'a str,
    pub disabled_reason: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub struct ExplorationLimits {
    pub campaign: CampaignLimits,
    pub max_choices: usize,
    pub max_text_bytes: usize,
    pub max_identifier_bytes: usize,
}

/// Bounded, localized, already audience-safe presentation. The owning client maps
/// accepted wire views into these props and maps emitted input into existing RPCs.
/// Binding/revision are the existing correlation values and grant no authority.
/// Pending/rejection strings describe supplied presentation state, never a receipt.
pub struct ExplorationView<'a> {
    pub binding: ClientBindingId,
    pub revision: SessionRevision,
    pub campaign: CampaignView<'a>,
    pub npc: Option<ExplorationNpc<'a>>,
    pub heading: &'a str,
    pub choices: &'a [ExplorationChoice<'a>],
    pub draft_label: &'a str,
    pub submit_label: &'a str,
    pub draft_offer_id: Option<&'a str>,
    pub pending: Option<&'a str>,
    pub rejection: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorationValidationError {
    Campaign(CampaignValidationError),
    EmptyText,
    ResourceLimit,
    DuplicateOffer,
    ConflictingFeedback,
    WrongBinding,
    StaleView,
    Disposed,
}
impl fmt::Display for ExplorationValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Campaign(error) => fmt::Display::fmt(error, f),
            Self::EmptyText => f.write_str("exploration presentation text is empty"),
            Self::ResourceLimit => f.write_str("exploration presentation exceeds owner limits"),
            Self::DuplicateOffer => f.write_str("exploration offered identifiers repeat"),
            Self::ConflictingFeedback => f.write_str("exploration feedback is ambiguous"),
            Self::WrongBinding => f.write_str("exploration view belongs to another binding"),
            Self::StaleView => f.write_str("exploration view is stale or duplicate"),
            Self::Disposed => f.write_str("exploration phase is disposed"),
        }
    }
}
impl std::error::Error for ExplorationValidationError {}

impl ExplorationView<'_> {
    pub fn validate(&self, limits: ExplorationLimits) -> Result<(), ExplorationValidationError> {
        self.campaign
            .validate(limits.campaign)
            .map_err(ExplorationValidationError::Campaign)?;
        if self.choices.len() > limits.max_choices {
            return Err(ExplorationValidationError::ResourceLimit);
        }
        if self.pending.is_some() && self.rejection.is_some() {
            return Err(ExplorationValidationError::ConflictingFeedback);
        }
        let text = |value: &str, maximum: usize| {
            if value.trim().is_empty() {
                Err(ExplorationValidationError::EmptyText)
            } else if value.len() > maximum {
                Err(ExplorationValidationError::ResourceLimit)
            } else {
                Ok(())
            }
        };
        for value in [self.heading, self.draft_label, self.submit_label] {
            text(value, limits.max_text_bytes)?;
        }
        for value in [self.pending, self.rejection].into_iter().flatten() {
            text(value, limits.max_text_bytes)?;
        }
        if let Some(npc) = &self.npc {
            for value in [npc.name, npc.context, npc.dialogue] {
                text(value, limits.max_text_bytes)?;
            }
        }
        let mut ids = BTreeSet::new();
        for choice in self.choices {
            text(choice.id, limits.max_identifier_bytes)?;
            text(choice.label, limits.max_text_bytes)?;
            text(choice.detail, limits.max_text_bytes)?;
            if let Some(reason) = choice.disabled_reason {
                text(reason, limits.max_text_bytes)?;
            }
            if !ids.insert(choice.id) {
                return Err(ExplorationValidationError::DuplicateOffer);
            }
        }
        if let Some(id) = self.draft_offer_id {
            text(id, limits.max_identifier_bytes)?;
            if !ids.insert(id) {
                return Err(ExplorationValidationError::DuplicateOffer);
            }
        }
        Ok(())
    }
}

/// Local input proposal only; no accepted result or inferred game outcome.
#[derive(Debug, Eq, PartialEq)]
pub enum ExplorationInput {
    Choice {
        id: String,
        revision: SessionRevision,
    },
    Draft {
        id: String,
        text: String,
        revision: SessionRevision,
    },
}

#[cfg(any(target_arch = "wasm32", test))]
struct OfferFence {
    binding: ClientBindingId,
    revision: SessionRevision,
    choices: Vec<(String, bool)>,
    draft: Option<String>,
    visible: bool,
    pending: bool,
    disposed: bool,
}
#[cfg(any(target_arch = "wasm32", test))]
impl OfferFence {
    fn create(view: &ExplorationView<'_>) -> Self {
        let mut state = Self {
            binding: view.binding,
            revision: view.revision,
            choices: Vec::new(),
            draft: None,
            visible: true,
            pending: false,
            disposed: false,
        };
        state.replace(view);
        state
    }
    fn require_newer(&self, view: &ExplorationView<'_>) -> Result<(), ExplorationValidationError> {
        if self.disposed {
            return Err(ExplorationValidationError::Disposed);
        }
        if view.binding != self.binding {
            return Err(ExplorationValidationError::WrongBinding);
        }
        if view.revision <= self.revision {
            return Err(ExplorationValidationError::StaleView);
        }
        Ok(())
    }
    fn replace(&mut self, view: &ExplorationView<'_>) {
        self.revision = view.revision;
        self.pending = view.pending.is_some();
        self.choices = view
            .choices
            .iter()
            .map(|choice| (choice.id.to_owned(), choice.disabled_reason.is_none()))
            .collect();
        self.draft = view.draft_offer_id.map(str::to_owned);
    }
    fn choice(&self, id: &str, epoch: u64) -> Option<ExplorationInput> {
        if !self.available(epoch)
            || !self
                .choices
                .iter()
                .any(|(offered, enabled)| offered == id && *enabled)
        {
            return None;
        }
        Some(ExplorationInput::Choice {
            id: id.to_owned(),
            revision: self.revision,
        })
    }
    fn draft(&self, text: String) -> Option<ExplorationInput> {
        if !self.available(self.revision.epoch().get()) || text.trim().is_empty() {
            return None;
        }
        self.draft.as_ref().map(|id| ExplorationInput::Draft {
            id: id.clone(),
            text,
            revision: self.revision,
        })
    }
    fn available(&self, epoch: u64) -> bool {
        !self.disposed && self.visible && !self.pending && self.revision.epoch().get() == epoch
    }
    fn dispose(&mut self) {
        self.disposed = true;
        self.choices.clear();
        self.draft = None;
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::{
        ActionView, CampaignError, CampaignSceneAssets, CampaignSurface, ControlError,
        ControlledAction, ControlledTextInput, DraftUpdate, InputFeedback, SceneImageLimits,
        TextInputView, UiError,
    };
    use df_client::cache::{CacheScope, FetchToken};
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
        rc::Rc,
    };
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{Document, Element, Event, HtmlElement, HtmlInputElement, Node};

    #[derive(Debug)]
    pub enum ExplorationError {
        InvalidView(ExplorationValidationError),
        Campaign(CampaignError),
        Control(ControlError),
        Dom(UiError),
    }
    impl fmt::Display for ExplorationError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidView(e) => fmt::Display::fmt(e, f),
                Self::Campaign(e) => fmt::Display::fmt(e, f),
                Self::Control(e) => fmt::Display::fmt(e, f),
                Self::Dom(e) => fmt::Display::fmt(e, f),
            }
        }
    }
    impl std::error::Error for ExplorationError {}
    impl From<wasm_bindgen::JsValue> for ExplorationError {
        fn from(e: wasm_bindgen::JsValue) -> Self {
            Self::Dom(UiError::Browser(e))
        }
    }
    impl From<ControlError> for ExplorationError {
        fn from(e: ControlError) -> Self {
            Self::Control(e)
        }
    }
    impl From<CampaignError> for ExplorationError {
        fn from(e: CampaignError) -> Self {
            Self::Campaign(e)
        }
    }
    impl From<ExplorationValidationError> for ExplorationError {
        fn from(e: ExplorationValidationError) -> Self {
            Self::InvalidView(e)
        }
    }

    type InputCallback = Box<dyn FnMut(ExplorationInput)>;
    struct ChoiceNodes {
        root: Element,
        action: ControlledAction,
        detail: Element,
        reason: Element,
    }
    impl ChoiceNodes {
        fn dispose(&self) -> Result<(), ExplorationError> {
            let mut failure = None;
            record_failure(&mut failure, erase_dom(&self.root));
            record_failure(
                &mut failure,
                self.action.dispose().map_err(ExplorationError::from),
            );
            self.root.remove();
            finish_cleanup(failure)
        }
    }

    fn record_failure(
        failure: &mut Option<ExplorationError>,
        result: Result<(), ExplorationError>,
    ) {
        if let Err(error) = result
            && failure.is_none()
        {
            *failure = Some(error);
        }
    }
    fn finish_cleanup(failure: Option<ExplorationError>) -> Result<(), ExplorationError> {
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    // Empty existing Text.data before any detach/replacement. set_text_content
    // alone leaves a previously retained Text reference readable.
    fn erase_text(root: &Node) {
        let mut pending = vec![root.clone()];
        while let Some(node) = pending.pop() {
            let mut child = node.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                pending.push(current);
            }
            node.set_node_value(Some(""));
        }
    }
    fn replace_text(element: &Element, value: Option<&str>) {
        erase_text(element);
        element.set_text_content(value);
    }
    fn erase_dom(root: &Element) -> Result<(), ExplorationError> {
        let mut failure = None;
        let mut pending: Vec<Node> = vec![root.clone().into()];
        while let Some(node) = pending.pop() {
            let mut child = node.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                pending.push(current);
            }
            node.set_node_value(Some(""));
            if let Some(input) = node.dyn_ref::<HtmlInputElement>() {
                input.set_value("");
            }
            if let Some(element) = node.dyn_ref::<Element>() {
                for name in element.get_attribute_names().iter() {
                    if let Some(name) = name.as_string() {
                        record_failure(
                            &mut failure,
                            element
                                .remove_attribute(&name)
                                .map_err(ExplorationError::from),
                        );
                    }
                }
            }
        }
        finish_cleanup(failure)
    }
    fn erase_campaign_replaced_text(surface: &CampaignSurface) -> Result<(), ExplorationError> {
        // Only dynamic audience fields replaced by CampaignSurface.update are
        // scrubbed; static branding, theme styles and caller-owned additions stay.
        for selector in [".topbar", ".stage", ".lower", "footer.connection"] {
            if let Some(root) = surface.root().query_selector(selector)? {
                let mut pending = vec![root];
                while let Some(element) = pending.pop() {
                    let mut child = element.first_element_child();
                    while let Some(current) = child {
                        child = current.next_element_sibling();
                        pending.push(current);
                    }
                    if element.matches(".chapter,.hero h1,.description,.location,.scene-panel h2,.narration blockquote,.connection,.preview-badge,.member-name,.member-role,.sigil span,.objective")? {
                        erase_text(&element);
                    }
                }
            }
        }
        Ok(())
    }
    struct ArtListener {
        target: Element,
        event_name: &'static str,
        active: Rc<Cell<bool>>,
        callback: Option<Closure<dyn FnMut(Event)>>,
    }
    impl ArtListener {
        fn bind(
            target: Element,
            event_name: &'static str,
            mut operation: impl FnMut() + 'static,
        ) -> Result<Self, ExplorationError> {
            let active = Rc::new(Cell::new(true));
            let live = Rc::clone(&active);
            let callback = Closure::wrap(Box::new(move |_| {
                if live.get() {
                    operation();
                }
            }) as Box<dyn FnMut(Event)>);
            target
                .add_event_listener_with_callback(event_name, callback.as_ref().unchecked_ref())?;
            Ok(Self {
                target,
                event_name,
                active,
                callback: Some(callback),
            })
        }
        fn dispose(&mut self) -> Result<(), ExplorationError> {
            self.active.set(false);
            if let Some(callback) = &self.callback {
                self.target.remove_event_listener_with_callback(
                    self.event_name,
                    callback.as_ref().unchecked_ref(),
                )?;
            }
            self.callback = None;
            Ok(())
        }
    }
    impl Drop for ArtListener {
        fn drop(&mut self) {
            if self.dispose().is_err()
                && let Some(callback) = self.callback.take()
            {
                callback.forget();
            }
        }
    }
    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
    ) -> Result<Element, ExplorationError> {
        let node = document.create_element(tag)?;
        node.set_class_name(class);
        parent.append_child(&node)?;
        Ok(node)
    }
    fn deliver(
        callback: &Rc<RefCell<Option<InputCallback>>>,
        fence: &Rc<RefCell<OfferFence>>,
        input: Option<ExplorationInput>,
    ) {
        if let Some(input) = input {
            let operation = callback.borrow_mut().take();
            if let Some(mut operation) = operation {
                operation(input);
                if !fence.borrow().disposed {
                    *callback.borrow_mut() = Some(operation);
                }
            }
        }
    }

    /// Stable cinematic exploration/dialogue composition. Each control/listener is
    /// owned once; generation replacement clears drafts and retires old choices.
    /// Call set_visible on shell suspension; activation also checks Document.hidden.
    pub struct ExplorationPhase {
        document: Document,
        surface: CampaignSurface,
        npc: Element,
        portrait: Element,
        npc_name: Element,
        npc_context: Element,
        dialogue: Element,
        heading: Element,
        choices_root: Element,
        empty: Element,
        field: Rc<ControlledTextInput>,
        submit: ControlledAction,
        choices: RefCell<BTreeMap<String, ChoiceNodes>>,
        fence: Rc<RefCell<OfferFence>>,
        callback: Rc<RefCell<Option<InputCallback>>>,
        callback_registered: Cell<bool>,
        listeners: RefCell<Vec<ArtListener>>,
        limits: ExplorationLimits,
    }
    impl ExplorationPhase {
        pub fn create(
            document: &Document,
            identifier: &str,
            view: &ExplorationView<'_>,
            limits: ExplorationLimits,
        ) -> Result<Self, ExplorationError> {
            view.validate(limits)?;
            let surface = CampaignSurface::create(document, &view.campaign, limits.campaign)?;
            surface.root().set_class_name("df-campaign df-exploration");
            let style = child(document, surface.root(), "style", "")?;
            style.set_text_content(Some(crate::exploration_phase_theme::STYLES));
            let interaction = child(
                document,
                surface.root(),
                "section",
                "exploration-interaction",
            )?;
            interaction.set_attribute("aria-label", "Scene interaction")?;
            if let Some(lower) = surface.root().query_selector(".lower")? {
                surface.root().insert_before(&interaction, Some(&lower))?;
            }
            // The existing narrator stays mounted; its scene overlay shares the stage's
            // audience-text erasure and CampaignSurface update ownership.
            if let Some(stage) = surface.root().query_selector(".stage")?
                && let Some(narration) = surface.root().query_selector(".narration")?
            {
                stage.append_child(&narration)?;
            }
            // Scene context belongs to the masthead's negative space, leaving the
            // conversational artwork unobstructed. The same mounted nodes are retained.
            if let Some(hero) = surface.root().query_selector(".hero")?
                && let Some(scene_panel) = surface.root().query_selector(".scene-panel")?
            {
                hero.append_child(&scene_panel)?;
            }
            let npc = child(document, &interaction, "article", "exploration-npc")?;
            let portrait = child(document, &npc, "img", "exploration-portrait")?;
            let identity = child(document, &npc, "div", "exploration-npc-text")?;
            let npc_name = child(document, &identity, "h2", "exploration-npc-name")?;
            let npc_context = child(document, &identity, "p", "exploration-context")?;
            let dialogue = child(document, &identity, "blockquote", "exploration-dialogue")?;
            let actions = child(document, &interaction, "section", "exploration-actions")?;
            let heading = child(document, &actions, "h2", "exploration-heading")?;
            let choices_root = child(document, &actions, "div", "exploration-choices")?;
            let empty = child(document, &actions, "p", "exploration-empty")?;
            empty.set_text_content(Some("No choices are currently offered."));
            let field = Rc::new(ControlledTextInput::create(
                document,
                identifier,
                TextInputView {
                    label: view.draft_label,
                    enabled: view.draft_offer_id.is_some(),
                    feedback: InputFeedback::None,
                },
                "",
            )?);
            actions.append_child(field.root())?;
            let submit = ControlledAction::create(
                document,
                ActionView {
                    label: view.submit_label,
                    enabled: false,
                    pending: false,
                },
            )?;
            submit.element().set_class_name("exploration-submit");
            actions.append_child(submit.element())?;
            let fence = Rc::new(RefCell::new(OfferFence::create(view)));
            let callback = Rc::new(RefCell::new(None));
            let draft_fence = Rc::clone(&fence);
            let draft_button = submit.element().clone();
            let draft_document = document.clone();
            field.on_change(move |result| {
                let can_submit = result.as_ref().is_ok_and(|text| !text.trim().is_empty())
                    && draft_fence.borrow().draft.is_some()
                    && draft_fence
                        .borrow()
                        .available(draft_fence.borrow().revision.epoch().get())
                    && !draft_document.hidden();
                draft_button.set_disabled(!can_submit);
            })?;
            let submit_fence = Rc::clone(&fence);
            let submit_field = Rc::clone(&field);
            let submit_callback = Rc::clone(&callback);
            let submit_document = document.clone();
            submit.on_activate(move || {
                if submit_document.hidden() {
                    return;
                }
                let input = submit_fence.borrow().draft(submit_field.draft());
                deliver(&submit_callback, &submit_fence, input);
            })?;
            let phase = Self {
                document: document.clone(),
                surface,
                npc,
                portrait,
                npc_name,
                npc_context,
                dialogue,
                heading,
                choices_root,
                empty,
                field,
                submit,
                choices: RefCell::new(BTreeMap::new()),
                fence,
                callback,
                callback_registered: Cell::new(false),
                listeners: RefCell::new(Vec::new()),
                limits,
            };
            let fallback = child(
                document,
                phase.surface.root(),
                "p",
                "exploration-art-fallback",
            )?;
            fallback.set_text_content(Some(
                "Scene art unavailable · narration and choices remain available.",
            ));
            if let Some(stage) = phase.surface.root().query_selector(".stage")? {
                phase
                    .surface
                    .root()
                    .insert_before(&fallback, Some(&stage))?;
            }
            let failed_portrait = phase.portrait.clone();
            phase.listeners.borrow_mut().push(ArtListener::bind(
                phase.portrait.clone(),
                "error",
                move || {
                    failed_portrait
                        .set_class_name("exploration-portrait exploration-portrait-failed");
                },
            )?);
            let narration = phase
                .surface
                .root()
                .query_selector(".narration")?
                .ok_or(ExplorationError::Dom(UiError::WrongElementType))?;
            let narrator = narration
                .query_selector(".narrator")?
                .ok_or(ExplorationError::Dom(UiError::WrongElementType))?;
            let narrator_fallback =
                child(document, &narration, "span", "narrator narrator-fallback")?;
            narrator_fallback.set_attribute("role", "img")?;
            narrator_fallback.set_attribute("aria-label", "Narrator portrait unavailable")?;
            narrator_fallback.set_text_content(Some("✦"));
            narration.insert_before(&narrator_fallback, Some(&narrator))?;
            let failed_narrator = narrator.clone();
            let visible_fallback = narrator_fallback.clone();
            phase.listeners.borrow_mut().push(ArtListener::bind(
                narrator.clone(),
                "error",
                move || {
                    failed_narrator.set_class_name("narrator narrator-failed");
                    visible_fallback
                        .set_class_name("narrator narrator-fallback narrator-fallback-visible");
                },
            )?);
            let recovered_narrator = narrator.clone();
            phase
                .listeners
                .borrow_mut()
                .push(ArtListener::bind(narrator, "load", move || {
                    recovered_narrator.set_class_name("narrator");
                    narrator_fallback.set_class_name("narrator narrator-fallback");
                })?);
            phase.render(view, DraftUpdate::Replace(""))?;
            Ok(phase)
        }
        pub fn root(&self) -> &Element {
            self.surface.root()
        }
        pub fn draft_input(&self) -> &ControlledTextInput {
            &self.field
        }
        pub fn on_input(
            &self,
            callback: impl FnMut(ExplorationInput) + 'static,
        ) -> Result<(), ExplorationError> {
            if self.fence.borrow().disposed {
                return Err(ExplorationValidationError::Disposed.into());
            }
            if self.callback_registered.replace(true) {
                return Err(ControlError::CallbackAlreadyRegistered.into());
            }
            *self.callback.borrow_mut() = Some(Box::new(callback));
            Ok(())
        }
        pub fn enable_scene_assets(
            &self,
            scope: CacheScope,
            limits: SceneImageLimits,
        ) -> Result<(), ExplorationError> {
            if self.fence.borrow().disposed {
                return Err(ExplorationValidationError::Disposed.into());
            }
            if scope.binding != self.fence.borrow().binding {
                return Err(ExplorationValidationError::WrongBinding.into());
            }
            self.surface
                .enable_scene_assets(scope, limits)
                .map_err(Into::into)
        }
        pub fn validate_scene_assets(
            &self,
            view: &ExplorationView<'_>,
            assets: &CampaignSceneAssets<'_>,
        ) -> Result<(), ExplorationError> {
            view.validate(self.limits)?;
            self.fence.borrow().require_newer(view)?;
            if assets.scope.binding != view.binding {
                return Err(ExplorationValidationError::WrongBinding.into());
            }
            if assets.revision != view.revision {
                return Err(ExplorationValidationError::StaleView.into());
            }
            self.surface
                .validate_scene_assets(*assets)
                .map_err(Into::into)
        }
        pub fn complete_scene_asset(
            &self,
            token: &FetchToken,
            bytes: Vec<u8>,
        ) -> Result<(), ExplorationError> {
            if self.fence.borrow().disposed {
                return Err(ExplorationValidationError::Disposed.into());
            }
            self.surface
                .complete_scene_asset(token, bytes)
                .map_err(Into::into)
        }
        pub fn update_with_scene_assets(
            &self,
            view: &ExplorationView<'_>,
            assets: CampaignSceneAssets<'_>,
        ) -> Result<Option<FetchToken>, ExplorationError> {
            self.validate_scene_assets(view, &assets)?;
            self.update_owned(view, Some(assets))
        }
        /// Caller supplies only newer accepted snapshots for this binding. Old or
        /// duplicate revisions are rejected before any node or draft is changed.
        pub fn update(&self, view: &ExplorationView<'_>) -> Result<(), ExplorationError> {
            view.validate(self.limits)?;
            self.fence.borrow().require_newer(view)?;
            self.update_owned(view, None).map(|_| ())
        }
        fn update_owned(
            &self,
            view: &ExplorationView<'_>,
            assets: Option<CampaignSceneAssets<'_>>,
        ) -> Result<Option<FetchToken>, ExplorationError> {
            let result = (|| {
                let changed_epoch = self.fence.borrow().revision.epoch() != view.revision.epoch();
                let changed_draft = self.fence.borrow().draft.as_deref() != view.draft_offer_id;
                if changed_epoch {
                    for nodes in self.choices.borrow_mut().values() {
                        nodes.dispose()?;
                    }
                    self.choices.borrow_mut().clear();
                }
                self.fence.borrow_mut().replace(view);
                self.render_owned(
                    view,
                    if changed_epoch || changed_draft {
                        DraftUpdate::Replace("")
                    } else {
                        DraftUpdate::Preserve
                    },
                    assets,
                )
            })();
            if result.is_err() {
                // An accepted snapshot cannot leave obsolete optional artwork visible
                // after any later UI failure. The existing legacy path fences it.
                self.surface.update(&view.campaign)?;
            }
            result
        }
        fn render(
            &self,
            view: &ExplorationView<'_>,
            draft: DraftUpdate<'_>,
        ) -> Result<(), ExplorationError> {
            self.render_owned(view, draft, None).map(|_| ())
        }
        fn render_owned(
            &self,
            view: &ExplorationView<'_>,
            draft: DraftUpdate<'_>,
            assets: Option<CampaignSceneAssets<'_>>,
        ) -> Result<Option<FetchToken>, ExplorationError> {
            erase_campaign_replaced_text(&self.surface)?;
            let fetch = match assets {
                Some(assets) => self
                    .surface
                    .update_with_scene_assets(&view.campaign, assets)?,
                None => {
                    self.surface.update(&view.campaign)?;
                    None
                }
            };
            // CampaignSurface owns current concept identity and image failure state.
            // Preserve its restored fallback flag and unrelated phase classes.
            replace_text(&self.heading, Some(view.heading));
            if let Some(npc) = &view.npc {
                self.npc.remove_attribute("hidden")?;
                replace_text(&self.npc_name, Some(npc.name));
                replace_text(&self.npc_context, Some(npc.context));
                replace_text(&self.dialogue, Some(npc.dialogue));
                match npc.portrait {
                    Some(portrait) => {
                        let source = portrait.asset_path();
                        if self.portrait.get_attribute("src").as_deref() != Some(source) {
                            self.portrait.set_class_name("exploration-portrait");
                            self.portrait.set_attribute("src", source)?;
                        }
                        self.portrait.set_attribute("alt", npc.name)?;
                        self.portrait.remove_attribute("hidden")?;
                    }
                    None => {
                        self.portrait.remove_attribute("src")?;
                        self.portrait.remove_attribute("alt")?;
                        self.portrait.set_attribute("hidden", "")?;
                    }
                }
            } else {
                self.npc.set_attribute("hidden", "")?;
                replace_text(&self.npc_name, None);
                replace_text(&self.npc_context, None);
                replace_text(&self.dialogue, None);
                self.portrait.remove_attribute("src")?;
                self.portrait.remove_attribute("alt")?;
            }
            let focused = self.document.active_element();
            let mut choices = self.choices.borrow_mut();
            let obsolete: Vec<_> = choices
                .keys()
                .filter(|id| !view.choices.iter().any(|choice| choice.id == id.as_str()))
                .cloned()
                .collect();
            for id in obsolete {
                if let Some(nodes) = choices.remove(&id) {
                    nodes.dispose()?;
                }
            }
            for choice in view.choices {
                if !choices.contains_key(choice.id) {
                    let root = child(
                        &self.document,
                        &self.choices_root,
                        "article",
                        "exploration-choice",
                    )?;
                    let action = ControlledAction::create(
                        &self.document,
                        ActionView {
                            label: choice.label,
                            enabled: choice.disabled_reason.is_none(),
                            pending: view.pending.is_some(),
                        },
                    )?;
                    action
                        .element()
                        .set_attribute("data-exploration-offer", choice.id)?;
                    root.append_child(action.element())?;
                    let detail = child(&self.document, &root, "p", "exploration-choice-detail")?;
                    let reason = child(&self.document, &root, "p", "exploration-choice-reason")?;
                    let id = choice.id.to_owned();
                    let epoch = view.revision.epoch().get();
                    let fence = Rc::clone(&self.fence);
                    let callback = Rc::clone(&self.callback);
                    let document = self.document.clone();
                    action.on_activate(move || {
                        if document.hidden() {
                            return;
                        }
                        let input = fence.borrow().choice(&id, epoch);
                        deliver(&callback, &fence, input);
                    })?;
                    choices.insert(
                        choice.id.to_owned(),
                        ChoiceNodes {
                            root,
                            action,
                            detail,
                            reason,
                        },
                    );
                }
                if let Some(nodes) = choices.get(choice.id) {
                    erase_text(nodes.action.element());
                    nodes.action.update(ActionView {
                        label: choice.label,
                        enabled: choice.disabled_reason.is_none() && self.fence.borrow().visible,
                        pending: view.pending.is_some(),
                    })?;
                    replace_text(&nodes.detail, Some(choice.detail));
                    replace_text(&nodes.reason, choice.disabled_reason);
                    // Reorder only changed positions; ordinary updates leave focus alone.
                    let previous = view
                        .choices
                        .iter()
                        .take_while(|offered| offered.id != choice.id)
                        .last();
                    let expected_previous = previous
                        .and_then(|offered| choices.get(offered.id))
                        .map(|entry| &entry.root);
                    let actual_previous = nodes.root.previous_element_sibling();
                    let positioned = match (expected_previous, actual_previous.as_ref()) {
                        (Some(expected), Some(actual)) => expected.is_same_node(Some(actual)),
                        (None, None) => true,
                        _ => false,
                    };
                    if !positioned {
                        let next = expected_previous
                            .and_then(Element::next_element_sibling)
                            .or_else(|| {
                                if expected_previous.is_none() {
                                    self.choices_root.first_element_child()
                                } else {
                                    None
                                }
                            });
                        self.choices_root
                            .insert_before(&nodes.root, next.as_ref().map(|node| node.as_ref()))?;
                    }
                }
            }
            if let Some(focused) = focused
                && self.surface.root().contains(Some(&focused))
                && !self
                    .document
                    .active_element()
                    .as_ref()
                    .is_some_and(|active| active.is_same_node(Some(&focused)))
                && let Some(element) = focused.dyn_ref::<HtmlElement>()
            {
                element.focus()?;
            }
            self.empty.set_attribute("hidden", "")?;
            if view.choices.is_empty() {
                self.empty.remove_attribute("hidden")?;
            }
            let feedback = if let Some(message) = view.pending {
                InputFeedback::Pending(message)
            } else if let Some(message) = view.rejection {
                InputFeedback::Rejected(message)
            } else {
                InputFeedback::None
            };
            // An absent offer is waiting presentation, while the original empty,
            // read-only control remains available for privacy/lifecycle verification.
            self.field.root().set_attribute(
                "data-draft-offered",
                if view.draft_offer_id.is_some() {
                    "true"
                } else {
                    "false"
                },
            )?;
            if view.draft_offer_id.is_some() {
                self.field.root().remove_attribute("hidden")?;
                self.submit.element().remove_attribute("hidden")?;
            } else {
                self.field.root().set_attribute("hidden", "")?;
                self.submit.element().set_attribute("hidden", "")?;
            }
            erase_text(self.field.root());
            self.field.update(
                TextInputView {
                    label: view.draft_label,
                    enabled: view.draft_offer_id.is_some() && self.fence.borrow().visible,
                    feedback,
                },
                draft,
            )?;
            erase_text(self.submit.element());
            self.submit.update(ActionView {
                label: view.submit_label,
                enabled: view.draft_offer_id.is_some()
                    && !self.field.draft().trim().is_empty()
                    && self.fence.borrow().visible,
                pending: view.pending.is_some(),
            })?;
            Ok(fetch)
        }
        /// Presentation suspension only. It grants no permissions on return; the
        /// shell must reconcile its current accepted server view before resuming.
        pub fn set_visible(&self, visible: bool) -> Result<(), ExplorationError> {
            if self.fence.borrow().disposed {
                return Err(ExplorationValidationError::Disposed.into());
            }
            self.fence.borrow_mut().visible = visible;
            if !visible {
                for nodes in self.choices.borrow().values() {
                    nodes.action.element().set_disabled(true);
                }
                self.submit.element().set_disabled(true);
                self.field.input().set_read_only(true);
            }
            Ok(())
        }
        /// Terminal and idempotent. Erases audience text, attributes and drafts
        /// in place before detaching, including references retained by a caller.
        /// Cleanup continues after a DOM failure and returns its first error.
        pub fn dispose(&self) -> Result<(), ExplorationError> {
            self.fence.borrow_mut().dispose();
            self.callback.borrow_mut().take();
            let mut failure = None;
            record_failure(&mut failure, erase_dom(self.surface.root()));
            for element in [
                &self.npc,
                &self.portrait,
                &self.npc_name,
                &self.npc_context,
                &self.dialogue,
                &self.heading,
                &self.choices_root,
                &self.empty,
            ] {
                record_failure(&mut failure, erase_dom(element));
            }
            // A reused control may already be detached by its caller.
            record_failure(&mut failure, erase_dom(self.field.root()));
            record_failure(&mut failure, erase_dom(self.submit.element()));
            record_failure(
                &mut failure,
                self.field.dispose().map_err(ExplorationError::from),
            );
            record_failure(
                &mut failure,
                self.submit.dispose().map_err(ExplorationError::from),
            );
            for nodes in self.choices.borrow().values() {
                record_failure(&mut failure, nodes.dispose());
            }
            self.choices.borrow_mut().clear();
            for listener in self.listeners.borrow_mut().iter_mut() {
                record_failure(&mut failure, listener.dispose());
            }
            self.listeners.borrow_mut().clear();
            record_failure(
                &mut failure,
                self.surface.dispose().map_err(ExplorationError::from),
            );
            self.surface.root().remove();
            finish_cleanup(failure)
        }
    }
    impl Drop for ExplorationPhase {
        fn drop(&mut self) {
            // Explicit disposal reports errors; Drop runs the same complete
            // erasure and callback fencing even when the caller omits it.
            if self.dispose().is_err() {
                self.surface.root().remove();
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::{ExplorationError, ExplorationPhase};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CampaignObjective, ConceptScene, ObjectiveState};
    use df_types::RecoveryEpoch;

    fn binding() -> ClientBindingId {
        ClientBindingId::from_bytes(&[1; 16]).expect("fixture binding")
    }
    fn revision(epoch: u64, sequence: u64) -> SessionRevision {
        SessionRevision::new(RecoveryEpoch::new(epoch).expect("fixture epoch"), sequence)
    }
    fn limits() -> ExplorationLimits {
        ExplorationLimits {
            campaign: CampaignLimits {
                max_members: 8,
                max_objectives: 8,
                max_text_bytes: 512,
            },
            max_choices: 8,
            max_text_bytes: 512,
            max_identifier_bytes: 256,
        }
    }
    const CHOICES: [ExplorationChoice<'static>; 2] = [
        ExplorationChoice {
            id: "opaque:/choice?ask=1",
            label: "Ask Vell",
            detail: "Ask about the lantern",
            disabled_reason: None,
        },
        ExplorationChoice {
            id: "unavailable-choice",
            label: "Leave",
            detail: "Return to the harbor",
            disabled_reason: Some("Await the server's current offer"),
        },
    ];
    const OBJECTIVES: [CampaignObjective<'static>; 1] = [CampaignObjective {
        label: "Speak to Vell",
        state: ObjectiveState::Active,
    }];
    fn view() -> ExplorationView<'static> {
        ExplorationView {
            binding: binding(),
            revision: revision(1, 1),
            campaign: CampaignView {
                scene: ConceptScene::Tavern,
                chapter: "Act I",
                title: "The Lantern",
                description: "Rain at the harbor",
                location: "Tavern",
                scene_label: "Conversation",
                narration: "A glass rests on the bar",
                connection: "Synthetic",
                notice: "Fixture",
                members: &[],
                objectives: &OBJECTIVES,
            },
            npc: Some(ExplorationNpc {
                name: "Vell",
                context: "Barkeeper",
                dialogue: "<svg onload=alert(1)>",
                portrait: Some(ExplorationPortrait::Vell),
            }),
            heading: "Offered choices",
            choices: &CHOICES,
            draft_label: "Your words",
            submit_label: "Send",
            draft_offer_id: Some("opaque:draft"),
            pending: None,
            rejection: None,
        }
    }
    #[test]
    fn bounded_props_reject_duplicate_opaque_offers_and_conflicting_feedback() {
        let mut data = view();
        assert_eq!(data.validate(limits()), Ok(()));
        data.draft_offer_id = Some(CHOICES[0].id);
        assert_eq!(
            data.validate(limits()),
            Err(ExplorationValidationError::DuplicateOffer)
        );
        data.draft_offer_id = None;
        data.pending = Some("Awaiting server");
        data.rejection = Some("Refused");
        assert_eq!(
            data.validate(limits()),
            Err(ExplorationValidationError::ConflictingFeedback)
        );
        data.rejection = None;
        assert_eq!(
            data.validate(ExplorationLimits {
                max_choices: 1,
                ..limits()
            }),
            Err(ExplorationValidationError::ResourceLimit)
        );
    }
    #[test]
    fn exact_offers_are_fenced_by_visibility_pending_and_generation() {
        let mut data = view();
        let mut fence = OfferFence::create(&data);
        assert_eq!(
            fence.choice(CHOICES[0].id, 1),
            Some(ExplorationInput::Choice {
                id: CHOICES[0].id.to_owned(),
                revision: data.revision
            })
        );
        assert_eq!(fence.choice(CHOICES[1].id, 1), None);
        assert_eq!(fence.choice("invented", 1), None);
        fence.visible = false;
        assert_eq!(fence.choice(CHOICES[0].id, 1), None);
        assert_eq!(fence.draft("editable draft".into()), None);
        fence.visible = true;
        data.pending = Some("Awaiting server");
        fence.replace(&data);
        assert_eq!(fence.choice(CHOICES[0].id, 1), None);
        data.pending = None;
        data.revision = revision(2, 0);
        fence.replace(&data);
        assert_eq!(fence.choice(CHOICES[0].id, 1), None);
        assert!(fence.choice(CHOICES[0].id, 2).is_some());
        fence.dispose();
        fence.dispose();
        assert_eq!(fence.choice(CHOICES[0].id, 2), None);
        assert_eq!(fence.draft("obsolete".into()), None);
    }
    #[test]
    fn stale_duplicate_and_foreign_views_cannot_mutate_current_offer_basis() {
        let mut data = view();
        let fence = OfferFence::create(&data);
        assert_eq!(
            fence.require_newer(&data),
            Err(ExplorationValidationError::StaleView)
        );
        data.revision = revision(1, 0);
        assert_eq!(
            fence.require_newer(&data),
            Err(ExplorationValidationError::StaleView)
        );
        data.revision = revision(2, 0);
        assert_eq!(fence.require_newer(&data), Ok(()));
        data.binding = ClientBindingId::from_bytes(&[2; 16]).expect("foreign fixture");
        assert_eq!(
            fence.require_newer(&data),
            Err(ExplorationValidationError::WrongBinding)
        );
        assert!(fence.choice(CHOICES[0].id, 1).is_some());
    }
    #[test]
    fn draft_is_an_exact_proposal_and_portrait_paths_are_closed() {
        let data = view();
        let fence = OfferFence::create(&data);
        assert_eq!(
            fence.draft("<b>my words</b>".into()),
            Some(ExplorationInput::Draft {
                id: "opaque:draft".into(),
                text: "<b>my words</b>".into(),
                revision: data.revision
            })
        );
        assert_eq!(fence.draft("  ".into()), None);
        assert_eq!(
            ExplorationPortrait::Vell.asset_path(),
            "assets/concept-art/vell-avatar.webp"
        );
    }
}
