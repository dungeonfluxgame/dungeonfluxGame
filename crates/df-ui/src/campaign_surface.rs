use std::{collections::BTreeSet, fmt};

/// Closed artwork selection. Baked interface references are never scene backgrounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConceptScene {
    Harbor,
    MaraHarbor,
    Tavern,
    Mountain,
    SunkenHall,
    Campfire,
    /// Generated scene paintings, served extensionless; a missing file shows the art-failed state.
    LobbyHarbor,
    WharfArrival,
    WhisperInTheRain,
    CourierRoad,
    HarborInn,
}

impl ConceptScene {
    pub const fn asset_path(self) -> &'static str {
        match self {
            Self::Harbor => "assets/concept-art/establishing-harbor-canal-rowboat-bridge.webp",
            Self::MaraHarbor => "assets/ui/scenes/mara-harbor-v4.png",
            Self::Tavern => "assets/concept-art/scene-tavern-barkeep-talk-rain.webp",
            Self::Mountain => "assets/concept-art/scene-mountain-ruins-snowy-ridge-trek.webp",
            Self::SunkenHall => "assets/concept-art/scene-flooded-hall-party-wading-torchlit.webp",
            Self::Campfire => "assets/concept-art/scene-campfire-under-stars.webp",
            Self::LobbyHarbor => "assets/ui/generated/scene-lobby-harbor",
            Self::WharfArrival => "assets/ui/generated/scene-wharf-arrival",
            Self::WhisperInTheRain => "assets/ui/generated/scene-whisper-in-the-rain",
            Self::CourierRoad => "assets/ui/generated/scene-courier-road",
            Self::HarborInn => "assets/ui/generated/scene-harbor-inn",
        }
    }
    pub const fn description(self) -> &'static str {
        match self {
            Self::Harbor => "Moonlit harbor with lanterns, a stone bridge and a distant castle",
            Self::MaraHarbor => "Mara in her navy coat beneath the lanterns of Greyhaven harbor",
            Self::Tavern => "Adventurers talking to a barkeeper in a candlelit tavern",
            Self::Mountain => "Adventurers trekking toward snowy mountain ruins",
            Self::SunkenHall => "A torchlit party wading through a flooded hall",
            Self::Campfire => "A campfire beneath the stars",
            Self::LobbyHarbor => "Rain-soaked harbor lanterns and a waiting tavern at night",
            Self::WharfArrival => "A hooded courier clutching a broken-sealed packet on the wharf",
            Self::WhisperInTheRain => "A rain-soaked wharf where a whispered warning is shared",
            Self::CourierRoad => "A lantern-lit road beside the harbor after the fight",
            Self::HarborInn => "A warm harbor inn with lamplight and rain on the windows",
        }
    }
}

/// Resource policy chosen by the mounting owner, separate from gameplay party limits.
#[derive(Clone, Copy, Debug)]
pub struct CampaignLimits {
    pub max_members: usize,
    pub max_objectives: usize,
    pub max_text_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectiveState {
    Pending,
    Active,
    Complete,
}

pub struct CampaignMember<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub role: &'a str,
    pub sigil: &'a str,
}
pub struct CampaignObjective<'a> {
    pub label: &'a str,
    pub state: ObjectiveState,
}

/// Already localized, audience-safe presentation from the caller. No mechanical
/// resolution, permission filtering, or connection inference happens in this model.
pub struct CampaignView<'a> {
    pub scene: ConceptScene,
    pub chapter: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub location: &'a str,
    pub scene_label: &'a str,
    pub narration: &'a str,
    pub connection: &'a str,
    pub notice: &'a str,
    pub members: &'a [CampaignMember<'a>],
    pub objectives: &'a [CampaignObjective<'a>],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CampaignValidationError {
    ResourceLimit,
    EmptyText,
    DuplicateMemberKey,
}
impl fmt::Display for CampaignValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResourceLimit => "campaign presentation exceeds owner limits",
            Self::EmptyText => "campaign presentation text is empty",
            Self::DuplicateMemberKey => "campaign member presentation keys repeat",
        })
    }
}
impl std::error::Error for CampaignValidationError {}

impl CampaignView<'_> {
    pub fn validate(&self, limits: CampaignLimits) -> Result<(), CampaignValidationError> {
        if self.members.len() > limits.max_members || self.objectives.len() > limits.max_objectives
        {
            return Err(CampaignValidationError::ResourceLimit);
        }
        let validate = |text: &str| {
            if text.trim().is_empty() {
                Err(CampaignValidationError::EmptyText)
            } else if text.len() > limits.max_text_bytes {
                Err(CampaignValidationError::ResourceLimit)
            } else {
                Ok(())
            }
        };
        for text in [
            self.chapter,
            self.title,
            self.description,
            self.location,
            self.scene_label,
            self.narration,
            self.connection,
            self.notice,
        ] {
            validate(text)?;
        }
        let mut keys = BTreeSet::new();
        for member in self.members {
            for text in [member.key, member.name, member.role, member.sigil] {
                validate(text)?;
            }
            if !keys.insert(member.key) {
                return Err(CampaignValidationError::DuplicateMemberKey);
            }
        }
        for objective in self.objectives {
            validate(objective.label)?;
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::scene_image::browser::SceneImageOwner;
    use crate::{CampaignSceneAssets, SceneImageError, SceneImageLimits, UiError};
    use df_client::cache::{CacheScope, FetchToken};
    use std::rc::Rc;
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
    };
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::Event;
    use web_sys::{Document, Element};

    #[derive(Debug)]
    pub enum CampaignError {
        InvalidView(CampaignValidationError),
        Dom(UiError),
        Disposed,
        SceneImage(SceneImageError),
    }
    impl fmt::Display for CampaignError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidView(e) => fmt::Display::fmt(e, f),
                Self::Dom(e) => fmt::Display::fmt(e, f),
                Self::Disposed => f.write_str("campaign surface is disposed"),
                Self::SceneImage(error) => write!(f, "scene image refused: {error:?}"),
            }
        }
    }
    impl std::error::Error for CampaignError {}
    impl From<wasm_bindgen::JsValue> for CampaignError {
        fn from(e: wasm_bindgen::JsValue) -> Self {
            Self::Dom(UiError::Browser(e))
        }
    }
    impl From<CampaignValidationError> for CampaignError {
        fn from(e: CampaignValidationError) -> Self {
            Self::InvalidView(e)
        }
    }

    type ArtworkCallback = Closure<dyn FnMut(Event)>;

    struct MemberNodes {
        root: Element,
        name: Element,
        role: Element,
        sigil: Element,
    }
    /// One mounted cinematic shell. Updates retain root, navigation controls and
    /// keyed party nodes. The caller owns navigation listeners and disposes them
    /// before disposing the shell. There is no detached work or audio playback.
    pub struct CampaignSurface {
        document: Document,
        root: Element,
        art: RefCell<Element>,
        toolbar: Element,
        chapter: Element,
        title: Element,
        description: Element,
        location: Element,
        scene_label: Element,
        narration: Element,
        connection: Element,
        notice: Element,
        party: Element,
        objectives: Element,
        members: RefCell<BTreeMap<String, MemberNodes>>,
        objective_nodes: RefCell<Vec<Element>>,
        limits: CampaignLimits,
        disposed: Cell<bool>,
        scene_assets: RefCell<Option<Rc<RefCell<SceneImageOwner>>>>,
        art_listener: RefCell<Option<ArtworkCallback>>,
        art_identity: Rc<RefCell<Option<Rc<()>>>>,
    }
    fn element(
        doc: &Document,
        tag: &str,
        class: &str,
        text: Option<&str>,
    ) -> Result<Element, CampaignError> {
        let node = doc.create_element(tag)?;
        node.set_class_name(class);
        node.set_text_content(text);
        Ok(node)
    }
    fn child(
        doc: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
        text: Option<&str>,
    ) -> Result<Element, CampaignError> {
        let node = element(doc, tag, class, text)?;
        parent.append_child(&node)?;
        Ok(node)
    }
    /// Illustration attachment for an existing visible scene. Text, layout and controls
    /// stay owned by the caller; this facade owns only its supplied fallback and lifecycle.
    pub struct CampaignIllustration {
        root: Element,
        fallback: Element,
        owner: RefCell<Option<Rc<RefCell<SceneImageOwner>>>>,
        disposed: Cell<bool>,
    }
    fn replace_image_owner(
        slot: &RefCell<Option<Rc<RefCell<SceneImageOwner>>>>,
        root: &Element,
        fallback: &Element,
        scope: CacheScope,
        limits: SceneImageLimits,
        existing_art_host: bool,
    ) -> Result<(), CampaignError> {
        let previous = slot.borrow().as_ref().cloned();
        if let Some(old) = &previous
            && old.borrow().scope() != scope
        {
            old.borrow_mut().dispose()?;
        }
        limits.validate().map_err(CampaignError::SceneImage)?;
        if let Some(old) = &previous {
            old.borrow_mut().dispose()?;
            old.borrow().ensure_drained()?;
        }
        let owner = SceneImageOwner::new(scope, limits, root, fallback)?;
        if existing_art_host {
            owner.borrow_mut().use_existing_art_host();
        }
        *slot.borrow_mut() = Some(owner);
        Ok(())
    }
    impl CampaignIllustration {
        pub fn mount_existing(root: &Element, fallback: &Element) -> Result<Self, CampaignError> {
            let document = root.owner_document().ok_or(CampaignError::Disposed)?;
            if fallback.dyn_ref::<web_sys::HtmlImageElement>().is_none()
                || !fallback
                    .owner_document()
                    .is_some_and(|owner| owner.is_same_node(Some(document.as_ref())))
                || !fallback
                    .parent_node()
                    .is_some_and(|parent| parent.is_same_node(Some(root.as_ref())))
            {
                return Err(CampaignError::Disposed);
            }
            Ok(Self {
                root: root.clone(),
                fallback: fallback.clone(),
                owner: RefCell::new(None),
                disposed: Cell::new(false),
            })
        }
        pub fn is_attached(&self) -> bool {
            self.fallback.parent_node().is_some()
                || self.root.get_attribute("data-scene-image").as_deref() == Some("generated")
        }
        pub fn attach(&self) -> Result<(), CampaignError> {
            if self.disposed.get() {
                return Err(CampaignError::Disposed);
            }
            if self.fallback.parent_node().is_none() {
                self.root.append_child(&self.fallback)?;
            }
            Ok(())
        }
        pub fn enable_scene_assets(
            &self,
            scope: CacheScope,
            limits: SceneImageLimits,
        ) -> Result<(), CampaignError> {
            if self.disposed.get() {
                return Err(CampaignError::Disposed);
            }
            replace_image_owner(&self.owner, &self.root, &self.fallback, scope, limits, true)
        }
        pub fn update_with_scene_assets(
            &self,
            scene: ConceptScene,
            assets: CampaignSceneAssets<'_>,
        ) -> Result<Option<FetchToken>, CampaignError> {
            if self.disposed.get() {
                return Err(CampaignError::Disposed);
            }
            let owner = self
                .owner
                .borrow()
                .as_ref()
                .cloned()
                .ok_or(CampaignError::SceneImage(SceneImageError::NotEnabled))?;
            owner.borrow().validate(assets)?;
            owner.borrow_mut().take_failure()?;
            let token = SceneImageOwner::reconcile(&owner, assets)?;
            // Exact closed ConceptScene paths remain the existing fallback contract.
            self.set_concept_fallback(scene)?;
            owner.borrow().set_description(scene.description());
            Ok(token)
        }
        /// Closed public concept fallback; it neither reopens a codec nor restores old pixels.
        pub fn set_concept_fallback(&self, scene: ConceptScene) -> Result<(), CampaignError> {
            if self.disposed.get() {
                return Err(CampaignError::Disposed);
            }
            self.fallback.set_attribute("alt", scene.description())?;
            let path = format!("/{}", scene.asset_path());
            if self.fallback.get_attribute("src").as_deref() != Some(path.as_str()) {
                self.fallback.set_attribute("src", &path)?;
            }
            if self.root.get_attribute("data-scene-image").as_deref() != Some("generated") {
                if self.fallback.parent_node().is_none() {
                    self.root.append_child(&self.fallback)?;
                }
                self.root.set_attribute("data-scene-image", "fallback")?;
            }
            Ok(())
        }
        pub fn complete_scene_asset(
            &self,
            token: &FetchToken,
            bytes: Vec<u8>,
        ) -> Result<(), CampaignError> {
            if self.disposed.get() {
                return Err(CampaignError::Disposed);
            }
            let owner = self
                .owner
                .borrow()
                .as_ref()
                .cloned()
                .ok_or(CampaignError::SceneImage(SceneImageError::NotEnabled))?;
            owner.borrow_mut().take_failure()?;
            SceneImageOwner::complete(&owner, token, bytes)
        }
        /// One bounded replay notification after the currently owned codec's terminal.
        /// False means no pending codec exists; no idle subscription is retained.
        pub fn after_decode_terminal(&self, callback: Box<dyn FnOnce()>) -> bool {
            if self.disposed.get() {
                return false;
            }
            self.owner
                .borrow()
                .as_ref()
                .is_some_and(|owner| owner.borrow_mut().after_terminal(callback))
        }
        pub fn take_scene_failure(&self) -> Result<(), CampaignError> {
            if self.disposed.get() {
                return Err(CampaignError::Disposed);
            }
            if let Some(owner) = self.owner.borrow().as_ref() {
                owner.borrow_mut().take_failure()?;
            }
            Ok(())
        }
        pub fn show_fallback(&self) -> Result<(), CampaignError> {
            if self.disposed.get() {
                return Err(CampaignError::Disposed);
            }
            if let Some(owner) = self.owner.borrow().as_ref() {
                owner.borrow_mut().legacy()?;
            }
            Ok(())
        }
        /// Keep the closed owner reachable while its real codec cancellation drains.
        pub fn retire(&self) -> Result<(), CampaignError> {
            if let Some(owner) = self.owner.borrow().as_ref() {
                owner.borrow_mut().dispose()?;
            }
            self.fallback.remove();
            self.fallback.remove_attribute("src")?;
            self.root.remove_attribute("data-scene-image")?;
            Ok(())
        }
        pub fn dispose(&self) -> Result<(), CampaignError> {
            if self.disposed.replace(true) {
                return Ok(());
            }
            self.retire()
        }
    }
    impl Drop for CampaignIllustration {
        fn drop(&mut self) {
            let _cleanup = self.dispose();
        }
    }
    impl CampaignSurface {
        pub fn create(
            document: &Document,
            view: &CampaignView<'_>,
            limits: CampaignLimits,
        ) -> Result<Self, CampaignError> {
            view.validate(limits)?;
            let root = element(document, "section", "df-campaign", None)?;
            let style = child(
                document,
                &root,
                "style",
                "",
                Some(crate::campaign_theme::STYLES),
            )?;
            style.set_attribute("data-campaign-theme", "")?;
            let art = child(document, &root, "img", "scene-art", None)?;
            art.set_attribute("fetchpriority", "high")?;
            let header = child(document, &root, "header", "topbar", None)?;
            let brand = child(document, &header, "div", "brand", Some("Dungeon"))?;
            child(document, &brand, "span", "", Some("Flux"))?;
            let toolbar = child(document, &header, "nav", "toolbar", None)?;
            toolbar.set_attribute("aria-label", "Presentation scenes")?;
            let notice = child(document, &header, "div", "preview-badge", None)?;
            let stage = child(document, &root, "div", "stage", None)?;
            let hero = child(document, &stage, "div", "hero", None)?;
            let chapter = child(document, &hero, "p", "chapter", None)?;
            let title = child(document, &hero, "h1", "", None)?;
            let description = child(document, &hero, "p", "description", None)?;
            let location = child(document, &hero, "p", "location", None)?;
            let panel = child(document, &stage, "aside", "scene-panel", None)?;
            child(document, &panel, "div", "overline", Some("Current scene"))?;
            let scene_label = child(document, &panel, "h2", "", None)?;
            let objectives = child(document, &panel, "ol", "objectives", None)?;
            let lower = child(document, &root, "div", "lower", None)?;
            let narration_panel = child(document, &lower, "section", "narration", None)?;
            narration_panel.set_attribute("aria-label", "Narration")?;
            let avatar = child(document, &narration_panel, "img", "narrator", None)?;
            avatar.set_attribute("src", "assets/concept-art/dm-avatar.webp")?;
            avatar.set_attribute("alt", "Hooded narrator portrait")?;
            let narrative = child(document, &narration_panel, "div", "", None)?;
            child(
                document,
                &narrative,
                "div",
                "overline",
                Some("Dungeon Master · Narration"),
            )?;
            let narration = child(document, &narrative, "blockquote", "", None)?;
            let party_section = child(document, &lower, "section", "party-section", None)?;
            let party_heading = child(document, &party_section, "div", "party-heading", None)?;
            child(
                document,
                &party_heading,
                "div",
                "overline",
                Some("Your party"),
            )?;
            child(
                document,
                &party_heading,
                "span",
                "overline",
                Some("Together, a story"),
            )?;
            let party = child(document, &party_section, "div", "party", None)?;
            let connection = child(document, &root, "footer", "connection", None)?;
            let surface = Self {
                document: document.clone(),
                root,
                art: RefCell::new(art),
                toolbar,
                chapter,
                title,
                description,
                location,
                scene_label,
                narration,
                connection,
                notice,
                party,
                objectives,
                members: RefCell::new(BTreeMap::new()),
                objective_nodes: RefCell::new(Vec::new()),
                limits,
                disposed: Cell::new(false),
                scene_assets: RefCell::new(None),
                art_listener: RefCell::new(None),
                art_identity: Rc::new(RefCell::new(None)),
            };
            surface.update(view)?;
            Ok(surface)
        }
        pub fn root(&self) -> &Element {
            &self.root
        }
        pub fn navigation(&self) -> &Element {
            &self.toolbar
        }
        pub fn update(&self, view: &CampaignView<'_>) -> Result<(), CampaignError> {
            if self.disposed.get() {
                return Err(CampaignError::Disposed);
            }
            view.validate(self.limits)?;
            if let Some(owner) = self.scene_assets.borrow().as_ref() {
                owner.borrow_mut().legacy()?;
            }
            self.update_content(view)
        }
        pub fn enable_scene_assets(
            &self,
            scope: CacheScope,
            limits: SceneImageLimits,
        ) -> Result<(), CampaignError> {
            if self.disposed.get() {
                return Err(CampaignError::Disposed);
            }
            replace_image_owner(
                &self.scene_assets,
                &self.root,
                &self.art.borrow(),
                scope,
                limits,
                false,
            )
        }
        pub(crate) fn validate_scene_assets(
            &self,
            assets: CampaignSceneAssets<'_>,
        ) -> Result<(), CampaignError> {
            if self.disposed.get() {
                return Err(CampaignError::Disposed);
            }
            self.scene_assets
                .borrow()
                .as_ref()
                .ok_or(CampaignError::SceneImage(SceneImageError::NotEnabled))?
                .borrow()
                .validate(assets)
        }
        pub fn update_with_scene_assets(
            &self,
            view: &CampaignView<'_>,
            assets: CampaignSceneAssets<'_>,
        ) -> Result<Option<FetchToken>, CampaignError> {
            view.validate(self.limits)?;
            self.validate_scene_assets(assets)?;
            let owner = self
                .scene_assets
                .borrow()
                .as_ref()
                .cloned()
                .ok_or(CampaignError::SceneImage(SceneImageError::NotEnabled))?;
            owner.borrow_mut().take_failure()?;
            let token = SceneImageOwner::reconcile(&owner, assets)?;
            if let Err(error) = self.update_content(view) {
                owner.borrow_mut().legacy()?;
                return Err(error);
            }
            Ok(token)
        }
        pub fn complete_scene_asset(
            &self,
            token: &FetchToken,
            bytes: Vec<u8>,
        ) -> Result<(), CampaignError> {
            if self.disposed.get() {
                return Err(CampaignError::Disposed);
            }
            let owner = self
                .scene_assets
                .borrow()
                .as_ref()
                .cloned()
                .ok_or(CampaignError::SceneImage(SceneImageError::NotEnabled))?;
            owner.borrow_mut().take_failure()?;
            SceneImageOwner::complete(&owner, token, bytes)
        }
        fn update_concept_art(&self, scene: ConceptScene) -> Result<(), CampaignError> {
            let old = self.art.borrow().clone();
            if old.get_attribute("src").as_deref() == Some(scene.asset_path()) {
                return Ok(());
            }
            // A separate browser image owns each concept request. An old network event
            // targets its old image even while its replacement is still loading.
            let next = element(&self.document, "img", "scene-art", None)?;
            next.set_attribute("alt", scene.description())?;
            next.set_attribute("fetchpriority", "high")?;
            let identity = Rc::new(());
            let callback_identity = identity.clone();
            let active = self.art_identity.clone();
            let fallback = next.clone();
            let root = self.root.clone();
            let source = scene.asset_path();
            let callback = Closure::wrap(Box::new(move |_: Event| {
                let current = active
                    .borrow()
                    .as_ref()
                    .is_some_and(|current| Rc::ptr_eq(current, &callback_identity));
                if current
                    && fallback.parent_node().is_some()
                    && fallback.get_attribute("src").as_deref() == Some(source)
                {
                    let classes = root.class_name();
                    if !classes
                        .split_whitespace()
                        .any(|name| name == "exploration-art-failed")
                    {
                        root.set_class_name(&format!("{classes} exploration-art-failed"));
                    }
                }
            }) as Box<dyn FnMut(Event)>);
            next.add_event_listener_with_callback("error", callback.as_ref().unchecked_ref())?;
            if let Some(parent) = old.parent_node()
                && let Err(error) = parent.replace_child(&next, &old)
            {
                next.remove_event_listener_with_callback(
                    "error",
                    callback.as_ref().unchecked_ref(),
                )?;
                return Err(error.into());
            }
            *self.art_identity.borrow_mut() = Some(identity);
            let old_listener = self.art_listener.borrow_mut().replace(callback);
            *self.art.borrow_mut() = next.clone();
            if let Some(owner) = self.scene_assets.borrow().as_ref() {
                owner.borrow_mut().replace_fallback(&next);
            }
            let mut failure = None;
            if let Some(listener) = old_listener
                && let Err(error) = old
                    .remove_event_listener_with_callback("error", listener.as_ref().unchecked_ref())
            {
                failure = Some(CampaignError::from(error));
            }
            old.remove();
            if let Err(error) = old.remove_attribute("src")
                && failure.is_none()
            {
                failure = Some(error.into());
            }
            let classes: Vec<_> = self
                .root
                .class_name()
                .split_whitespace()
                .filter(|name| *name != "exploration-art-failed")
                .map(str::to_owned)
                .collect();
            self.root.set_class_name(&classes.join(" "));
            next.set_attribute("src", source)?;
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
        fn update_content(&self, view: &CampaignView<'_>) -> Result<(), CampaignError> {
            self.update_concept_art(view.scene)?;
            if let Some(owner) = self.scene_assets.borrow().as_ref() {
                owner.borrow().set_description(view.scene.description());
            }
            for (node, text) in [
                (&self.chapter, view.chapter),
                (&self.title, view.title),
                (&self.description, view.description),
                (&self.location, view.location),
                (&self.scene_label, view.scene_label),
                (&self.narration, view.narration),
                (&self.connection, view.connection),
                (&self.notice, view.notice),
            ] {
                node.set_text_content(Some(text));
            }
            let mut members = self.members.borrow_mut();
            let obsolete: Vec<_> = members
                .keys()
                .filter(|key| !view.members.iter().any(|member| member.key == key.as_str()))
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(nodes) = members.remove(&key) {
                    self.party.remove_child(&nodes.root)?;
                }
            }
            for member in view.members {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    members.entry(member.key.to_owned())
                {
                    let root = element(&self.document, "article", "party-card", None)?;
                    let seal = child(&self.document, &root, "div", "sigil", None)?;
                    let sigil = child(&self.document, &seal, "span", "", None)?;
                    let content = child(&self.document, &root, "div", "", None)?;
                    let name = child(&self.document, &content, "div", "member-name", None)?;
                    let role = child(&self.document, &content, "div", "member-role", None)?;
                    entry.insert(MemberNodes {
                        root,
                        name,
                        role,
                        sigil,
                    });
                }
                if let Some(nodes) = members.get(member.key) {
                    nodes.name.set_text_content(Some(member.name));
                    nodes.role.set_text_content(Some(member.role));
                    nodes.sigil.set_text_content(Some(member.sigil));
                    self.party.append_child(&nodes.root)?;
                }
            }
            let mut objectives = self.objective_nodes.borrow_mut();
            while objectives.len() > view.objectives.len() {
                if let Some(node) = objectives.pop() {
                    self.objectives.remove_child(&node)?;
                }
            }
            while objectives.len() < view.objectives.len() {
                objectives.push(child(
                    &self.document,
                    &self.objectives,
                    "li",
                    "objective",
                    None,
                )?);
            }
            for (node, objective) in objectives.iter().zip(view.objectives) {
                node.set_text_content(Some(objective.label));
                node.set_attribute(
                    "data-state",
                    match objective.state {
                        ObjectiveState::Pending => "pending",
                        ObjectiveState::Active => "active",
                        ObjectiveState::Complete => "complete",
                    },
                )?;
            }
            Ok(())
        }
        /// Terminal, repeatable removal of artwork URLs, leases and listeners, then the shell.
        pub fn dispose(&self) -> Result<(), CampaignError> {
            self.disposed.set(true);
            let mut failure = None;
            if let Some(owner) = self.scene_assets.borrow_mut().take()
                && let Err(error) = owner.borrow_mut().dispose()
            {
                failure = Some(error);
            }
            self.art_identity.borrow_mut().take();
            if let Some(listener) = self.art_listener.borrow_mut().take()
                && let Err(error) = self
                    .art
                    .borrow()
                    .remove_event_listener_with_callback("error", listener.as_ref().unchecked_ref())
                && failure.is_none()
            {
                failure = Some(error.into());
            }
            self.root.remove();
            if let Err(error) = self.art.borrow().remove_attribute("src")
                && failure.is_none()
            {
                failure = Some(error.into());
            }
            self.members.borrow_mut().clear();
            self.objective_nodes.borrow_mut().clear();
            self.root.set_text_content(None);
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }
    impl Drop for CampaignSurface {
        fn drop(&mut self) {
            let _cleanup = self.dispose();
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::{CampaignError, CampaignIllustration, CampaignSurface};

#[cfg(test)]
mod tests {
    use super::*;
    fn view<'a>(members: &'a [CampaignMember<'a>]) -> CampaignView<'a> {
        CampaignView {
            scene: ConceptScene::Harbor,
            chapter: "I",
            title: "<img src=x onerror=alert(1)>",
            description: "Description",
            location: "Harbor",
            scene_label: "Scene",
            narration: "Narration",
            connection: "Offline",
            notice: "Preview",
            members,
            objectives: &[],
        }
    }
    #[test]
    fn owner_limits_bound_presentation_without_a_four_player_rule() {
        let keys: Vec<_> = (0..8).map(|index| format!("member-{index}")).collect();
        let mut members: Vec<_> = keys
            .iter()
            .map(|key| CampaignMember {
                key,
                name: "Name",
                role: "Role",
                sigil: "N",
            })
            .collect();
        let limits = CampaignLimits {
            max_members: 8,
            max_objectives: 10,
            max_text_bytes: 100,
        };
        assert_eq!(view(&members).validate(limits), Ok(()));
        members[1].key = members[0].key;
        assert_eq!(
            view(&members).validate(limits),
            Err(CampaignValidationError::DuplicateMemberKey)
        );
        assert_eq!(view(&[]).validate(limits), Ok(()));
        assert_eq!(
            view(&members).validate(CampaignLimits {
                max_members: 7,
                ..limits
            }),
            Err(CampaignValidationError::ResourceLimit)
        );
    }
    #[test]
    fn empty_or_overlong_text_is_rejected_before_dom_publication() {
        let limits = CampaignLimits {
            max_members: 10,
            max_objectives: 10,
            max_text_bytes: 100,
        };
        let mut data = view(&[]);
        data.narration = " ";
        assert_eq!(
            data.validate(limits),
            Err(CampaignValidationError::EmptyText)
        );
        data.narration = "Narration";
        assert_eq!(
            data.validate(CampaignLimits {
                max_text_bytes: 3,
                ..limits
            }),
            Err(CampaignValidationError::ResourceLimit)
        );
    }
}
