//! Authored, audience-safe encounter aftermath. Presentation selections are not
//! game receipts, rewards or permission grants; the caller owns their admission.

use std::{collections::BTreeSet, fmt};

pub struct AftermathPartyMember<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub summary: &'a str,
    /// Optional hit-point bar on the member card (ratio only; the numbers are shown as given).
    pub hit_points: Option<crate::CombatHitPoints>,
    /// Optional short ASCII token (letters, digits, `-`) published as `data-portrait` so an
    /// owner stylesheet can attach portrait art; the member's initial is the fallback.
    pub portrait: Option<&'a str>,
}

#[derive(Clone, Copy)]
pub struct AftermathNextScene<'a> {
    pub key: &'a str,
    pub label: &'a str,
    pub enabled: bool,
    pub pending: bool,
}

/// Generation and revision belong to the presentation owner, not a game clock.
/// Both roles must receive independently filtered props before mounting this view.
pub struct EncounterAftermathView<'a> {
    pub generation: u64,
    pub revision: u64,
    pub chapter: &'a str,
    pub title: &'a str,
    pub narrative: &'a str,
    pub dialogue: Option<&'a str>,
    pub party_heading: &'a str,
    pub party: &'a [AftermathPartyMember<'a>],
    pub next_heading: &'a str,
    pub next_context: &'a str,
    pub next_empty: &'a str,
    pub next_scene: Option<AftermathNextScene<'a>>,
    pub feedback: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AftermathSelection {
    pub generation: u64,
    pub revision: u64,
    pub key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AftermathValidationError {
    EmptyText,
    TextLimit,
    PartyLimit,
    DuplicateMember,
    WrongGeneration,
    StaleView,
    Disposed,
}

impl fmt::Display for AftermathValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyText => "aftermath text is empty",
            Self::TextLimit => "aftermath text exceeds its bound",
            Self::PartyLimit => "aftermath party exceeds its bound",
            Self::DuplicateMember => "aftermath member keys repeat",
            Self::WrongGeneration => "aftermath owner was replaced",
            Self::StaleView => "aftermath view is stale or duplicate",
            Self::Disposed => "aftermath surface is disposed",
        })
    }
}
impl std::error::Error for AftermathValidationError {}

impl EncounterAftermathView<'_> {
    pub fn validate(&self) -> Result<(), AftermathValidationError> {
        if self.party.len() > 8 {
            return Err(AftermathValidationError::PartyLimit);
        }
        let mut total = 0usize;
        let mut text = |value: &str, bound: usize| {
            if value.trim().is_empty() {
                return Err(AftermathValidationError::EmptyText);
            }
            if value.len() > bound {
                return Err(AftermathValidationError::TextLimit);
            }
            total = total
                .checked_add(value.len())
                .ok_or(AftermathValidationError::TextLimit)?;
            if total > 16_384 {
                return Err(AftermathValidationError::TextLimit);
            }
            Ok(())
        };
        for value in [
            self.chapter,
            self.title,
            self.narrative,
            self.party_heading,
            self.next_heading,
            self.next_context,
            self.next_empty,
        ] {
            text(value, 4096)?;
        }
        if let Some(dialogue) = self.dialogue {
            text(dialogue, 4096)?;
        }
        let mut keys = BTreeSet::new();
        for member in self.party {
            text(member.key, 128)?;
            text(member.name, 256)?;
            if !member.summary.is_empty() {
                text(member.summary, 2048)?;
            }
            if let Some(portrait) = member.portrait
                && (portrait.is_empty()
                    || portrait.len() > 32
                    || !portrait
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
            {
                return Err(AftermathValidationError::EmptyText);
            }
            if !keys.insert(member.key) {
                return Err(AftermathValidationError::DuplicateMember);
            }
        }
        if let Some(next) = self.next_scene {
            text(next.key, 128)?;
            text(next.label, 256)?;
        }
        if let Some(feedback) = self.feedback {
            text(feedback, 2048)?;
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::{ActionView, ControlError, ControlledAction};
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
    use wasm_bindgen::JsValue;
    use web_sys::{Document, Element, Node};

    const STYLES: &str = r#"
.df-aftermath{position:relative;isolation:isolate;min-height:100svh;box-sizing:border-box;container-type:inline-size;padding:clamp(22px,4vw,64px);color:#f3e8d3;background:linear-gradient(180deg,#11171375,#111713ed 65%,#111713),url('assets/concept-art/combat-harbor-docks-wide-lanterns.webp') center 38%/cover,#111713;font:16px/1.65 Georgia,serif}
.df-aftermath *{box-sizing:border-box;min-width:0;overflow-wrap:anywhere}.df-aftermath [hidden]{display:none!important}.df-aftermath .aftermath-story{max-width:760px;margin:clamp(50px,16svh,170px) auto 34px}.df-aftermath .aftermath-chapter{margin:0 0 16px;font:11px/1.6 system-ui,sans-serif;letter-spacing:2px;text-transform:uppercase;color:#edcb8d}.df-aftermath h1{font-size:clamp(36px,5vw,68px);line-height:1.06;font-weight:400;letter-spacing:-1px;margin:0 0 23px;text-wrap:balance;text-shadow:0 3px 25px #000}.df-aftermath .aftermath-narrative{font-size:20px;line-height:1.75;margin:0;color:#eee2ca;text-shadow:0 2px 10px #000}.df-aftermath .aftermath-dialogue{font:20px/1.75 Georgia,serif;margin:16px 0 0;color:#eee2ca;text-shadow:0 2px 10px #000}.df-aftermath .aftermath-dialogue:empty,.df-aftermath .aftermath-member p:empty{display:none}.df-aftermath .aftermath-lower{display:grid;grid-template-columns:minmax(0,1.2fr) minmax(0,1fr);gap:24px;max-width:1000px;margin:auto}.df-aftermath .aftermath-panel{padding:24px;border:1px solid #c8a76b66;border-radius:8px;background:linear-gradient(135deg,#242b22f2,#151d17ef);box-shadow:0 16px 45px #0004}.df-aftermath h2{font-size:24px;font-weight:400;color:#ecd19f;margin:0 0 17px}.df-aftermath .aftermath-party{list-style:none;padding:0;margin:0}.df-aftermath .aftermath-member{display:flex;gap:14px;align-items:center;padding:12px 0;border-top:1px solid #b69b663d}.df-aftermath .aftermath-member-body{flex:1}.df-aftermath .aftermath-portrait{position:relative;flex:none;overflow:hidden;display:grid;place-items:center;width:56px;height:56px;border-radius:50%;border:2px solid #cba466b3;background:#162230;color:#e3c68f;font-size:24px}.df-aftermath .aftermath-hp{position:relative;height:16px;margin-top:6px;border:1px solid #d9b77a59;border-radius:8px;background:#0a0e0b;overflow:hidden}.df-aftermath .aftermath-hp-fill{position:absolute;inset:0 auto 0 0;width:var(--hp,0%);background:linear-gradient(90deg,#3f8553,#8fd18d)}.df-aftermath .aftermath-hp[data-state=low] .aftermath-hp-fill{background:linear-gradient(90deg,#92392a,#e0784f)}.df-aftermath .aftermath-hp-text{position:relative;display:block;text-align:center;font:700 11px/16px system-ui,sans-serif;color:#fff;text-shadow:0 1px 3px #000}.df-aftermath .aftermath-member:first-child{border-top:0;padding-top:0}.df-aftermath h3{font-size:19px;font-weight:400;margin:0 0 3px}.df-aftermath .aftermath-member p,.df-aftermath .aftermath-next-context{font:13px/1.7 system-ui,sans-serif;color:#d7cbb3;margin:0}.df-aftermath button{width:100%;min-height:48px;margin-top:20px;padding:12px 16px;border:1px solid #f0d195;border-radius:5px;background:linear-gradient(135deg,#efd3a0,#c6a15f);color:#211b11;font:600 14px/1.5 system-ui,sans-serif;cursor:pointer;touch-action:manipulation}.df-aftermath button:hover:enabled{background:#f5dcad}.df-aftermath button:disabled{color:#bdb49e;background:#20271f;border-color:#777d6459;cursor:default}.df-aftermath button[aria-busy=true]{border-style:dashed;color:#efd3a0;background:#313728;cursor:wait}.df-aftermath :focus-visible{outline:3px solid #ffe0a3;outline-offset:4px}.df-aftermath .aftermath-empty,.df-aftermath .aftermath-feedback{font:13px/1.7 system-ui,sans-serif;color:#ebcf9d;margin:14px 0 0}.df-aftermath .aftermath-feedback:empty{display:none}
@container(max-width:600px){.df-aftermath .aftermath-lower{grid-template-columns:1fr;gap:16px}.df-aftermath .aftermath-story{margin-top:55px}.df-aftermath h1{font-size:37px}.df-aftermath .aftermath-narrative,.df-aftermath .aftermath-dialogue{font-size:16px}.df-aftermath .aftermath-panel{padding:19px}}
"#;

    #[derive(Debug)]
    pub enum AftermathError {
        Validation(AftermathValidationError),
        Control(ControlError),
        Browser(JsValue),
    }
    impl fmt::Display for AftermathError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Validation(error) => fmt::Display::fmt(error, formatter),
                Self::Control(error) => fmt::Display::fmt(error, formatter),
                Self::Browser(_) => formatter.write_str("aftermath browser operation failed"),
            }
        }
    }
    impl std::error::Error for AftermathError {}
    impl From<AftermathValidationError> for AftermathError {
        fn from(error: AftermathValidationError) -> Self {
            Self::Validation(error)
        }
    }
    impl From<ControlError> for AftermathError {
        fn from(error: ControlError) -> Self {
            Self::Control(error)
        }
    }
    impl From<JsValue> for AftermathError {
        fn from(error: JsValue) -> Self {
            Self::Browser(error)
        }
    }

    type Callback = Box<dyn FnMut(AftermathSelection)>;
    struct Dispatch {
        generation: u64,
        revision: Option<u64>,
        disposed: bool,
        selection: Option<AftermathSelection>,
        callback: Option<Callback>,
    }
    struct MemberNodes {
        root: Element,
        portrait: Element,
        name: Element,
        summary: Element,
        hit_points: Element,
        hit_points_text: Element,
    }
    pub struct EncounterAftermathSurface {
        document: Document,
        root: Element,
        chapter: Element,
        title: Element,
        narrative: Element,
        dialogue: Element,
        party_heading: Element,
        party_root: Element,
        members: RefCell<BTreeMap<String, MemberNodes>>,
        next_heading: Element,
        next_context: Element,
        next_empty: Element,
        next_action: ControlledAction,
        feedback: Element,
        dispatch: Rc<RefCell<Dispatch>>,
    }
    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
    ) -> Result<Element, AftermathError> {
        let node = document.create_element(tag)?;
        node.set_class_name(class);
        parent.append_child(&node)?;
        Ok(node)
    }
    fn set_text(node: &Element, value: &str) {
        if node.text_content().as_deref() != Some(value) {
            scrub(node);
            node.set_text_content(Some(value));
        }
    }
    fn scrub(node: &Node) {
        let mut next = node.first_child();
        while let Some(child) = next {
            next = child.next_sibling();
            scrub(&child);
        }
        node.set_node_value(Some(""));
        node.set_text_content(None);
    }
    impl EncounterAftermathSurface {
        pub fn create(
            document: &Document,
            view: &EncounterAftermathView<'_>,
            on_next_scene: impl FnMut(AftermathSelection) + 'static,
        ) -> Result<Self, AftermathError> {
            view.validate()?;
            let root = document.create_element("section")?;
            root.set_class_name("df-aftermath");
            child(document, &root, "style", "")?.set_text_content(Some(STYLES));
            let story = child(document, &root, "header", "aftermath-story")?;
            let chapter = child(document, &story, "p", "aftermath-chapter")?;
            let title = child(document, &story, "h1", "")?;
            let narrative = child(document, &story, "p", "aftermath-narrative")?;
            let dialogue = child(document, &story, "p", "aftermath-dialogue")?;
            let lower = child(document, &root, "div", "aftermath-lower")?;
            let party = child(document, &lower, "section", "aftermath-panel")?;
            let party_heading = child(document, &party, "h2", "")?;
            let party_root = child(document, &party, "ul", "aftermath-party")?;
            let next = child(document, &lower, "section", "aftermath-panel")?;
            let next_heading = child(document, &next, "h2", "")?;
            let next_context = child(document, &next, "p", "aftermath-next-context")?;
            let next_empty = child(document, &next, "p", "aftermath-empty")?;
            let next_action = ControlledAction::create(
                document,
                ActionView {
                    label: view.next_empty,
                    enabled: false,
                    pending: false,
                },
            )?;
            next.append_child(next_action.element())?;
            let feedback = child(document, &next, "p", "aftermath-feedback")?;
            feedback.set_attribute("role", "status")?;
            feedback.set_attribute("aria-live", "polite")?;
            let dispatch = Rc::new(RefCell::new(Dispatch {
                generation: view.generation,
                revision: None,
                disposed: false,
                selection: None,
                callback: Some(Box::new(on_next_scene)),
            }));
            let listener_dispatch = Rc::clone(&dispatch);
            next_action.on_activate(move || {
                let selected = {
                    let mut owner = listener_dispatch.borrow_mut();
                    if owner.disposed {
                        return;
                    }
                    owner.selection.clone().and_then(|selection| {
                        owner.callback.take().map(|callback| (selection, callback))
                    })
                };
                if let Some((selection, mut callback)) = selected {
                    callback(selection);
                    let mut owner = listener_dispatch.borrow_mut();
                    if !owner.disposed {
                        owner.callback = Some(callback);
                    }
                }
            })?;
            let surface = Self {
                document: document.clone(),
                root,
                chapter,
                title,
                narrative,
                dialogue,
                party_heading,
                party_root,
                members: RefCell::new(BTreeMap::new()),
                next_heading,
                next_context,
                next_empty,
                next_action,
                feedback,
                dispatch,
            };
            surface.update(view)?;
            Ok(surface)
        }
        pub fn root(&self) -> &Element {
            &self.root
        }
        /// A newer accepted presentation updates the existing action and keyed
        /// party nodes. Render failure fences the next selection until recovery.
        pub fn update(&self, view: &EncounterAftermathView<'_>) -> Result<(), AftermathError> {
            view.validate()?;
            {
                let mut owner = self.dispatch.borrow_mut();
                if owner.disposed {
                    return Err(AftermathValidationError::Disposed.into());
                }
                if owner.generation != view.generation {
                    return Err(AftermathValidationError::WrongGeneration.into());
                }
                if owner
                    .revision
                    .is_some_and(|revision| view.revision <= revision)
                {
                    return Err(AftermathValidationError::StaleView.into());
                }
                owner.revision = Some(view.revision);
                owner.selection = None;
            }
            self.next_action.element().set_disabled(true);
            for (node, text) in [
                (&self.chapter, view.chapter),
                (&self.title, view.title),
                (&self.narrative, view.narrative),
                (&self.party_heading, view.party_heading),
                (&self.next_heading, view.next_heading),
                (&self.next_context, view.next_context),
                (&self.feedback, view.feedback.unwrap_or("")),
            ] {
                set_text(node, text);
            }
            set_text(&self.dialogue, view.dialogue.unwrap_or(""));
            self.publish_party(view.party)?;
            if let Some(next) = view.next_scene {
                self.publish_action(ActionView {
                    label: next.label,
                    enabled: next.enabled,
                    pending: next.pending,
                })?;
                self.next_action.element().remove_attribute("hidden")?;
                self.next_empty.set_attribute("hidden", "")?;
                self.next_empty.set_text_content(None);
                if next.enabled && !next.pending {
                    self.dispatch.borrow_mut().selection = Some(AftermathSelection {
                        generation: view.generation,
                        revision: view.revision,
                        key: next.key.to_owned(),
                    });
                }
            } else {
                self.publish_action(ActionView {
                    label: view.next_empty,
                    enabled: false,
                    pending: false,
                })?;
                self.next_action.element().set_attribute("hidden", "")?;
                self.next_empty.remove_attribute("hidden")?;
                set_text(&self.next_empty, view.next_empty);
            }
            Ok(())
        }
        fn publish_action(&self, view: ActionView<'_>) -> Result<(), AftermathError> {
            let retired = self.next_action.element().first_child();
            let updated = self.next_action.update(view);
            if let Some(node) = retired
                && !self.next_action.element().contains(Some(&node))
            {
                scrub(&node);
            }
            updated?;
            Ok(())
        }
        fn publish_party(&self, party: &[AftermathPartyMember<'_>]) -> Result<(), AftermathError> {
            let mut members = self.members.borrow_mut();
            let obsolete: Vec<_> = members
                .keys()
                .filter(|key| !party.iter().any(|member| member.key == key.as_str()))
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(member) = members.remove(&key) {
                    scrub(&member.root);
                    self.party_root.remove_child(&member.root)?;
                }
            }
            for (position, member) in party.iter().enumerate() {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    members.entry(member.key.to_owned())
                {
                    let root = self.document.create_element("li")?;
                    root.set_class_name("aftermath-member");
                    let portrait = child(&self.document, &root, "span", "aftermath-portrait")?;
                    let body = child(&self.document, &root, "div", "aftermath-member-body")?;
                    let name = child(&self.document, &body, "h3", "")?;
                    let summary = child(&self.document, &body, "p", "")?;
                    let hit_points = child(&self.document, &body, "div", "aftermath-hp")?;
                    hit_points.set_attribute("role", "meter")?;
                    hit_points.set_attribute("aria-label", "Hit points")?;
                    child(&self.document, &hit_points, "span", "aftermath-hp-fill")?;
                    let hit_points_text =
                        child(&self.document, &hit_points, "span", "aftermath-hp-text")?;
                    entry.insert(MemberNodes {
                        root,
                        portrait,
                        name,
                        summary,
                        hit_points,
                        hit_points_text,
                    });
                }
                if let Some(nodes) = members.get(member.key) {
                    set_text(&nodes.name, member.name);
                    set_text(&nodes.summary, member.summary);
                    set_text(
                        &nodes.portrait,
                        &member
                            .name
                            .chars()
                            .next()
                            .map(|initial| initial.to_uppercase().collect::<String>())
                            .unwrap_or_default(),
                    );
                    match member.portrait {
                        Some(portrait) => nodes.root.set_attribute("data-portrait", portrait)?,
                        None => nodes.root.remove_attribute("data-portrait")?,
                    }
                    match member.hit_points {
                        Some(points) => {
                            nodes.hit_points.remove_attribute("hidden")?;
                            // Presentation ratio only; the supplied numbers are shown unchanged.
                            let percent = if points.maximum == 0 {
                                0
                            } else {
                                u64::from(points.current.min(points.maximum)) * 100
                                    / u64::from(points.maximum)
                            };
                            nodes
                                .hit_points
                                .set_attribute("style", &format!("--hp:{percent}%"))?;
                            nodes.hit_points.set_attribute(
                                "data-state",
                                if points.current == 0 {
                                    "down"
                                } else if percent <= 35 {
                                    "low"
                                } else {
                                    "healthy"
                                },
                            )?;
                            nodes
                                .hit_points
                                .set_attribute("aria-valuenow", &points.current.to_string())?;
                            nodes
                                .hit_points
                                .set_attribute("aria-valuemax", &points.maximum.to_string())?;
                            set_text(
                                &nodes.hit_points_text,
                                &format!("{} / {} HP", points.current, points.maximum),
                            );
                        }
                        None => nodes.hit_points.set_attribute("hidden", "")?,
                    }
                    let mut before = self.party_root.first_child();
                    for _ in 0..position {
                        before = before.and_then(|node| node.next_sibling());
                    }
                    if !before
                        .as_ref()
                        .is_some_and(|node| node.is_same_node(Some(&nodes.root)))
                    {
                        self.party_root
                            .insert_before(&nodes.root, before.as_ref())?;
                    }
                }
            }
            Ok(())
        }
        /// Offline/reconnect does not authorize next-scene submission. A newer
        /// accepted view must supply a fresh enabled offer before input resumes.
        pub fn suspend_input(&self) -> Result<(), AftermathError> {
            let mut owner = self.dispatch.borrow_mut();
            if owner.disposed {
                return Err(AftermathValidationError::Disposed.into());
            }
            owner.selection = None;
            drop(owner);
            self.next_action.element().set_disabled(true);
            Ok(())
        }
        pub fn dispose(&self) -> Result<(), AftermathError> {
            {
                let mut owner = self.dispatch.borrow_mut();
                if owner.disposed {
                    return Ok(());
                }
                owner.disposed = true;
                owner.selection = None;
                owner.callback = None;
            }
            let control_result = self.next_action.dispose();
            scrub(&self.root);
            self.members.borrow_mut().clear();
            let removed = if let Some(parent) = self.root.parent_node() {
                parent.remove_child(&self.root).map(|_| ())
            } else {
                Ok(())
            };
            control_result?;
            removed?;
            Ok(())
        }
    }
    impl Drop for EncounterAftermathSurface {
        fn drop(&mut self) {
            if self.dispose().is_err() {
                scrub(&self.root);
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::{AftermathError, EncounterAftermathSurface};
