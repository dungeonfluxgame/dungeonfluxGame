use crate::character_phase::{CharacterPortrait, CharacterStatus, CharacterValidationError};
use std::collections::BTreeSet;

/// Public readiness reported by the server. Private build rejection is deliberately
/// absent from this shared-display projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharacterPublicReadiness {
    Choosing,
    Reviewing,
    Ready,
    Locked,
}
impl CharacterPublicReadiness {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Choosing => CharacterStatus::Editing.label(),
            Self::Reviewing => CharacterStatus::Pending.label(),
            Self::Ready => CharacterStatus::Ready.label(),
            Self::Locked => CharacterStatus::Locked.label(),
        }
    }
    #[cfg(target_arch = "wasm32")]
    const fn key(self) -> &'static str {
        match self {
            Self::Choosing => "choosing",
            Self::Reviewing => "reviewing",
            Self::Ready => "ready",
            Self::Locked => "locked",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharacterDisplayConnection {
    Connected,
    Reconnecting,
    Offline,
}

/// Only public values filtered by the server may enter this roster. There are no
/// private name/flavor/appearance drafts, build choices, scores, or rejection fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterPublicMember {
    pub key: String,
    pub character_name: String,
    pub portrait: Option<CharacterPortrait>,
    pub readiness: CharacterPublicReadiness,
    pub progress_label: String,
    /// Separately filtered public description; never copied from a player draft.
    pub appearance_summary: Option<String>,
}

/// An explicitly permitted host offer, already filtered by the server. An empty
/// list mounts no host controls; being a shared display never grants host rights.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterDisplayHostOffer {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub pending: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct CharacterDisplayLimits {
    pub max_members: usize,
    pub max_host_offers: usize,
    pub max_text_bytes: usize,
}

/// A separate audience-safe presentation projection, never derived in-browser
/// from CharacterPhaseView. Progress/readiness/count labels arrive from the server;
/// the client does not compute a build or decide when the party is ready.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterDisplayView {
    pub generation: u64,
    pub public_scope_key: String,
    pub revision: u64,
    pub chapter: String,
    pub title: String,
    pub description: String,
    pub readiness: CharacterPublicReadiness,
    pub progress_label: String,
    pub public_notice: String,
    pub connection: CharacterDisplayConnection,
    pub connection_label: String,
    pub members: Vec<CharacterPublicMember>,
    pub host_offers: Vec<CharacterDisplayHostOffer>,
}

/// Presentation intent only. The caller maps the advertised host offer through
/// its authorized production boundary; a callback never advances game state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterDisplaySubmission {
    pub generation: u64,
    pub public_scope_key: String,
    pub revision: u64,
    pub host_offer_id: String,
}

impl CharacterDisplayView {
    pub fn validate(&self, limits: CharacterDisplayLimits) -> Result<(), CharacterValidationError> {
        if self.members.len() > limits.max_members
            || self.host_offers.len() > limits.max_host_offers
        {
            return Err(CharacterValidationError::ResourceLimit);
        }
        let text = |value: &str, empty_allowed: bool| {
            if value.len() > limits.max_text_bytes {
                return Err(CharacterValidationError::ResourceLimit);
            }
            if (!empty_allowed && value.trim().is_empty()) || value.chars().any(char::is_control) {
                return Err(CharacterValidationError::InvalidText);
            }
            Ok(())
        };
        for value in [
            &self.public_scope_key,
            &self.chapter,
            &self.title,
            &self.description,
            &self.progress_label,
            &self.connection_label,
        ] {
            text(value, false)?;
        }
        text(&self.public_notice, true)?;
        let mut keys = BTreeSet::new();
        for member in &self.members {
            for value in [&member.key, &member.character_name, &member.progress_label] {
                text(value, false)?;
            }
            if let Some(summary) = &member.appearance_summary {
                text(summary, false)?;
            }
            if !keys.insert(&member.key) {
                return Err(CharacterValidationError::DuplicateId);
            }
        }
        let mut offers = BTreeSet::new();
        for offer in &self.host_offers {
            text(&offer.id, false)?;
            text(&offer.label, false)?;
            if !offers.insert(&offer.id) {
                return Err(CharacterValidationError::DuplicateId);
            }
        }
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
struct DisplayState {
    view: Option<CharacterDisplayView>,
}
#[cfg(any(target_arch = "wasm32", test))]
impl DisplayState {
    fn reconcile(&mut self, view: &CharacterDisplayView) -> Result<bool, CharacterValidationError> {
        let current = self
            .view
            .as_ref()
            .ok_or(CharacterValidationError::Disposed)?;
        let replaced = current.generation != view.generation
            || current.public_scope_key != view.public_scope_key;
        if !replaced
            && (view.revision < current.revision
                || (view.revision == current.revision && view != current))
        {
            return Err(CharacterValidationError::StaleRevision);
        }
        self.view = Some(view.clone());
        Ok(replaced)
    }
    fn submission(
        &self,
        generation: u64,
        scope: &str,
        id: &str,
    ) -> Result<CharacterDisplaySubmission, CharacterValidationError> {
        let view = self
            .view
            .as_ref()
            .ok_or(CharacterValidationError::Disposed)?;
        if generation != view.generation || scope != view.public_scope_key {
            return Err(CharacterValidationError::Unavailable);
        }
        let offer = view
            .host_offers
            .iter()
            .find(|offer| offer.id == id)
            .ok_or(CharacterValidationError::UnknownSelection)?;
        if view.connection != CharacterDisplayConnection::Connected
            || !offer.enabled
            || offer.pending
        {
            return Err(CharacterValidationError::Unavailable);
        }
        Ok(CharacterDisplaySubmission {
            generation,
            public_scope_key: scope.into(),
            revision: view.revision,
            host_offer_id: id.into(),
        })
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::character_phase::{
        CharacterPhaseError, PortraitSlot, place, replace_text, scrub_private_dom,
    };
    use crate::{ActionView, ControlledAction};
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
    use wasm_bindgen::JsCast;
    use web_sys::{Document, Element, HtmlElement};

    struct MemberNodes {
        root: Element,
        name: Element,
        readiness: Element,
        progress: Element,
        appearance: Element,
        portrait: PortraitSlot,
    }
    type HostCallback = Box<dyn FnMut(CharacterDisplaySubmission)>;
    /// Persistent shared-display creation presentation. The owner supplies only
    /// public projections; no private player form is mounted or hidden in this DOM.
    pub struct CharacterDisplaySurface {
        document: Document,
        root: Element,
        chapter: Element,
        title: Element,
        description: Element,
        readiness: Element,
        progress: Element,
        notice: Element,
        connection: Element,
        roster: Element,
        host_root: Element,
        members: RefCell<BTreeMap<String, MemberNodes>>,
        offers: RefCell<BTreeMap<String, ControlledAction>>,
        state: Rc<RefCell<DisplayState>>,
        callback: Rc<RefCell<Option<HostCallback>>>,
        limits: CharacterDisplayLimits,
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
    impl CharacterDisplaySurface {
        pub fn create(
            document: &Document,
            view: &CharacterDisplayView,
            limits: CharacterDisplayLimits,
            on_host_offer: impl FnMut(CharacterDisplaySubmission) + 'static,
        ) -> Result<Self, CharacterPhaseError> {
            view.validate(limits)?;
            let root = document.create_element("section")?;
            root.set_class_name("df-character df-character-display");
            child(
                document,
                &root,
                "style",
                "",
                Some(crate::character_phase_theme::STYLES),
            )?;
            child(
                document,
                &root,
                "style",
                "",
                Some(crate::character_display_phase_theme::STYLES),
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
                Some("Shared display · Public party view"),
            )?;
            let hero = child(document, &root, "div", "character-hero", None)?;
            let chapter = child(document, &hero, "p", "character-overline", None)?;
            let title = child(document, &hero, "h1", "", None)?;
            let description = child(document, &hero, "p", "character-description", None)?;
            let report = child(document, &hero, "div", "display-report", None)?;
            let readiness = child(document, &report, "p", "display-readiness", None)?;
            let progress = child(document, &report, "p", "display-progress", None)?;
            let notice = child(document, &hero, "p", "display-notice", None)?;
            notice.set_attribute("role", "status")?;
            notice.set_attribute("aria-live", "polite")?;
            let roster = child(document, &root, "div", "display-roster", None)?;
            roster.set_attribute("aria-label", "Public party character creation roster")?;
            let host_root = child(document, &root, "nav", "display-host-offers", None)?;
            host_root.set_attribute("aria-label", "Server-permitted host offers")?;
            let connection = child(document, &root, "footer", "character-connection", None)?;
            let surface = Self {
                document: document.clone(),
                root,
                chapter,
                title,
                description,
                readiness,
                progress,
                notice,
                connection,
                roster,
                host_root,
                members: RefCell::new(BTreeMap::new()),
                offers: RefCell::new(BTreeMap::new()),
                state: Rc::new(RefCell::new(DisplayState {
                    view: Some(view.clone()),
                })),
                callback: Rc::new(RefCell::new(Some(Box::new(on_host_offer)))),
                limits,
            };
            surface.update(view)?;
            Ok(surface)
        }
        pub fn root(&self) -> &Element {
            &self.root
        }
        pub fn update(&self, view: &CharacterDisplayView) -> Result<(), CharacterPhaseError> {
            view.validate(self.limits)?;
            let replaced = self.state.borrow_mut().reconcile(view)?;
            match self.render(view, replaced) {
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
        fn render(
            &self,
            view: &CharacterDisplayView,
            replaced: bool,
        ) -> Result<(), CharacterPhaseError> {
            let focused = self
                .document
                .active_element()
                .filter(|node| self.root.contains(Some(node)));
            for (node, text) in [
                (&self.chapter, view.chapter.as_str()),
                (&self.title, &view.title),
                (&self.description, &view.description),
                (&self.readiness, view.readiness.label()),
                (&self.progress, &view.progress_label),
                (&self.notice, &view.public_notice),
                (&self.connection, &view.connection_label),
            ] {
                replace_text(node, Some(text));
            }
            self.root
                .set_attribute("data-readiness", view.readiness.key())?;
            self.root
                .set_attribute("data-generation", &view.generation.to_string())?;
            self.root.set_attribute(
                "data-connection",
                match view.connection {
                    CharacterDisplayConnection::Connected => "connected",
                    CharacterDisplayConnection::Reconnecting => "reconnecting",
                    CharacterDisplayConnection::Offline => "offline",
                },
            )?;
            let mut members = self.members.borrow_mut();
            let obsolete: Vec<_> = members
                .keys()
                .filter(|key| replaced || !view.members.iter().any(|member| member.key == **key))
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(member) = members.remove(&key) {
                    scrub_private_dom(member.root.as_ref());
                    member.portrait.dispose()?;
                    member.root.remove_attribute("data-member-key")?;
                    self.roster.remove_child(&member.root)?;
                }
            }
            let mut cursor = self.roster.first_element_child();
            for member in &view.members {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    members.entry(member.key.clone())
                {
                    let root = self.document.create_element("article")?;
                    root.set_class_name("display-member");
                    root.set_attribute("data-member-key", &member.key)?;
                    let portrait = PortraitSlot::create(
                        &self.document,
                        &root,
                        "character-portrait display-portrait",
                    )?;
                    let name = child(&self.document, &root, "h2", "display-member-name", None)?;
                    let readiness =
                        child(&self.document, &root, "p", "display-member-readiness", None)?;
                    let progress =
                        child(&self.document, &root, "p", "display-member-progress", None)?;
                    let appearance = child(
                        &self.document,
                        &root,
                        "p",
                        "display-member-appearance",
                        None,
                    )?;
                    entry.insert(MemberNodes {
                        root,
                        name,
                        readiness,
                        progress,
                        appearance,
                        portrait,
                    });
                }
                if let Some(nodes) = members.get(&member.key) {
                    replace_text(&nodes.name, Some(&member.character_name));
                    replace_text(&nodes.readiness, Some(member.readiness.label()));
                    replace_text(&nodes.progress, Some(&member.progress_label));
                    replace_text(&nodes.appearance, member.appearance_summary.as_deref());
                    if member.appearance_summary.is_some() {
                        nodes.appearance.remove_attribute("hidden")?;
                    } else {
                        nodes.appearance.set_attribute("hidden", "")?;
                    }
                    nodes
                        .root
                        .set_attribute("data-readiness", member.readiness.key())?;
                    nodes
                        .portrait
                        .update(&self.document, member.portrait, replaced)?;
                    place(&self.roster, &nodes.root, &mut cursor)?;
                }
            }
            drop(members);
            self.render_offers(view, replaced)?;
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
        fn render_offers(
            &self,
            view: &CharacterDisplayView,
            replaced: bool,
        ) -> Result<(), CharacterPhaseError> {
            let mut offers = self.offers.borrow_mut();
            let obsolete: Vec<_> = offers
                .keys()
                .filter(|key| replaced || !view.host_offers.iter().any(|offer| offer.id == **key))
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(action) = offers.remove(&key) {
                    scrub_private_dom(action.element().as_ref());
                    action.dispose()?;
                    action.element().remove_attribute("data-host-offer-id")?;
                }
            }
            let mut cursor = self.host_root.first_element_child();
            for offer in &view.host_offers {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    offers.entry(offer.id.clone())
                {
                    let action = ControlledAction::create(
                        &self.document,
                        ActionView {
                            label: &offer.label,
                            enabled: offer.enabled,
                            pending: offer.pending,
                        },
                    )?;
                    action
                        .element()
                        .set_attribute("data-host-offer-id", &offer.id)?;
                    let state = Rc::clone(&self.state);
                    let callback = Rc::clone(&self.callback);
                    let generation = view.generation;
                    let scope = view.public_scope_key.clone();
                    let id = offer.id.clone();
                    let notice = self.notice.clone();
                    action.on_activate(move || {
                        let result = state.borrow().submission(generation, &scope, &id);
                        match result {
                            Ok(request) => {
                                let owned = callback.borrow_mut().take();
                                if let Some(mut operation) = owned {
                                    operation(request);
                                    if state.borrow().view.is_some() {
                                        *callback.borrow_mut() = Some(operation);
                                    }
                                }
                            }
                            Err(error) => replace_text(&notice, Some(&error.to_string())),
                        }
                    })?;
                    entry.insert(action);
                }
                if let Some(action) = offers.get(&offer.id) {
                    scrub_private_dom(action.element().as_ref());
                    action.update(ActionView {
                        label: &offer.label,
                        enabled: offer.enabled
                            && view.connection == CharacterDisplayConnection::Connected,
                        pending: offer.pending,
                    })?;
                    place(&self.host_root, action.element().as_ref(), &mut cursor)?;
                }
            }
            Ok(())
        }
        pub fn retry_failed_portraits(&self) -> Result<(), CharacterPhaseError> {
            if self.state.borrow().view.is_none() {
                return Err(CharacterValidationError::Disposed.into());
            }
            let result = (|| -> Result<(), CharacterPhaseError> {
                for nodes in self.members.borrow().values() {
                    nodes.portrait.retry(&self.document)?;
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
        /// Terminal and best-effort complete: callback fencing precedes DOM cleanup.
        pub fn dispose(&self) -> Result<(), CharacterPhaseError> {
            self.state.borrow_mut().view = None;
            self.callback.borrow_mut().take();
            scrub_private_dom(self.root.as_ref());
            for node in [
                &self.chapter,
                &self.title,
                &self.description,
                &self.readiness,
                &self.progress,
                &self.notice,
                &self.connection,
            ] {
                scrub_private_dom(node.as_ref());
            }
            let mut failure = None;
            let mut record = |result: Result<(), CharacterPhaseError>| {
                if let Err(error) = result
                    && failure.is_none()
                {
                    failure = Some(error);
                }
            };
            for member in self.members.borrow().values() {
                for node in [
                    &member.root,
                    &member.name,
                    &member.readiness,
                    &member.progress,
                    &member.appearance,
                ] {
                    scrub_private_dom(node.as_ref());
                }
                record(member.portrait.dispose());
                record(
                    member
                        .root
                        .remove_attribute("data-member-key")
                        .map_err(Into::into),
                );
            }
            for action in self.offers.borrow().values() {
                scrub_private_dom(action.element().as_ref());
                record(action.dispose().map_err(Into::into));
                record(
                    action
                        .element()
                        .remove_attribute("data-host-offer-id")
                        .map_err(Into::into),
                );
            }
            self.members.borrow_mut().clear();
            self.offers.borrow_mut().clear();
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
    impl Drop for CharacterDisplaySurface {
        fn drop(&mut self) {
            if self.dispose().is_err() {
                self.root.remove();
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::CharacterDisplaySurface;

#[cfg(test)]
mod tests {
    use super::*;
    fn view() -> CharacterDisplayView {
        CharacterDisplayView {
            generation: 1,
            public_scope_key: "public-room".into(),
            revision: 1,
            chapter: "Public prologue".into(),
            title: "A party takes shape".into(),
            description: "Public party view".into(),
            readiness: CharacterPublicReadiness::Choosing,
            progress_label: "Server-provided progress".into(),
            public_notice: String::new(),
            connection: CharacterDisplayConnection::Connected,
            connection_label: "Connected".into(),
            members: vec![CharacterPublicMember {
                key: "public-member".into(),
                character_name: "Mara".into(),
                portrait: None,
                readiness: CharacterPublicReadiness::Choosing,
                progress_label: "Public progress".into(),
                appearance_summary: None,
            }],
            host_offers: vec![CharacterDisplayHostOffer {
                id: "offered-host-action".into(),
                label: "Advertised action".into(),
                enabled: true,
                pending: false,
            }],
        }
    }
    #[test]
    fn public_roster_bounds_and_plain_text_do_not_impose_party_rules() {
        let limits = CharacterDisplayLimits {
            max_members: 12,
            max_host_offers: 8,
            max_text_bytes: 100,
        };
        let mut display_view = view();
        display_view.title = "<img src=x onerror=alert(1)>".into();
        assert_eq!(display_view.validate(limits), Ok(()));
        display_view.members.push(display_view.members[0].clone());
        assert_eq!(
            display_view.validate(limits),
            Err(CharacterValidationError::DuplicateId)
        );
        display_view.members.pop();
        display_view.public_notice = "bad\u{0}text".into();
        assert_eq!(
            display_view.validate(limits),
            Err(CharacterValidationError::InvalidText)
        );
        display_view.public_notice.clear();
        display_view.members[0].appearance_summary = Some("A silver braid and blue coat".into());
        assert_eq!(display_view.validate(limits), Ok(()));
        display_view.members[0].appearance_summary = Some("private\nline".into());
        assert_eq!(
            display_view.validate(limits),
            Err(CharacterValidationError::InvalidText)
        );
        display_view.members[0].appearance_summary = Some("x".repeat(101));
        assert_eq!(
            display_view.validate(limits),
            Err(CharacterValidationError::ResourceLimit)
        );
        display_view.members[0].appearance_summary = None;
        assert_eq!(
            display_view.validate(CharacterDisplayLimits {
                max_members: 0,
                ..limits
            }),
            Err(CharacterValidationError::ResourceLimit)
        );
        display_view.members.clear();
        display_view.host_offers.clear();
        assert_eq!(display_view.validate(limits), Ok(()));
    }
    #[test]
    fn only_advertised_connected_host_offers_emit_input_without_changing_readiness() {
        let mut display_view = view();
        let mut display_state = DisplayState {
            view: Some(display_view.clone()),
        };
        let request = display_state
            .submission(1, "public-room", "offered-host-action")
            .expect("advertised");
        assert_eq!(request.revision, 1);
        assert_eq!(
            display_state.view.as_ref().map(|view| view.readiness),
            Some(CharacterPublicReadiness::Choosing)
        );
        assert_eq!(
            display_state.submission(1, "public-room", "invented"),
            Err(CharacterValidationError::UnknownSelection)
        );
        for connection in [
            CharacterDisplayConnection::Offline,
            CharacterDisplayConnection::Reconnecting,
        ] {
            display_view.connection = connection;
            display_view.revision += 1;
            display_state
                .reconcile(&display_view)
                .expect("current view");
            assert_eq!(
                display_state.submission(1, "public-room", "offered-host-action"),
                Err(CharacterValidationError::Unavailable)
            );
        }
        display_view.connection = CharacterDisplayConnection::Connected;
        display_view.host_offers[0].pending = true;
        display_view.revision += 1;
        display_state
            .reconcile(&display_view)
            .expect("pending view");
        assert_eq!(
            display_state.submission(1, "public-room", "offered-host-action"),
            Err(CharacterValidationError::Unavailable)
        );
    }
    #[test]
    fn stale_equal_conflict_generation_offer_and_disposal_boundaries_are_fenced() {
        let mut display_view = view();
        let mut display_state = DisplayState {
            view: Some(display_view.clone()),
        };
        assert_eq!(display_state.reconcile(&display_view), Ok(false));
        let mut conflict = display_view.clone();
        conflict.readiness = CharacterPublicReadiness::Locked;
        assert_eq!(
            display_state.reconcile(&conflict),
            Err(CharacterValidationError::StaleRevision)
        );
        conflict.revision = 0;
        assert_eq!(
            display_state.reconcile(&conflict),
            Err(CharacterValidationError::StaleRevision)
        );
        display_view.generation = 2;
        display_view.public_scope_key = "new-public-room".into();
        display_view.revision = 0;
        assert_eq!(display_state.reconcile(&display_view), Ok(true));
        assert_eq!(
            display_state.submission(1, "public-room", "offered-host-action"),
            Err(CharacterValidationError::Unavailable)
        );
        display_view.host_offers.clear();
        display_view.revision += 1;
        display_state
            .reconcile(&display_view)
            .expect("revoked view");
        assert_eq!(
            display_state.submission(2, "new-public-room", "offered-host-action"),
            Err(CharacterValidationError::UnknownSelection)
        );
        display_state.view = None;
        assert_eq!(
            display_state.reconcile(&display_view),
            Err(CharacterValidationError::Disposed)
        );
    }
}
