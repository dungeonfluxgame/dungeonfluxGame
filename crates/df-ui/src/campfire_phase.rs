use std::{collections::BTreeSet, fmt};

/// A supplied condition summary, already filtered for this client audience.
/// Resource values and rest eligibility are never calculated by this component.
pub struct CampfireMember<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub condition: &'a str,
    pub detail: &'a str,
}

pub struct CampfireOffer<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub enabled: bool,
    pub pending: bool,
}

/// Presentation props, not a server model, RPC or grant. The owner changes
/// generation whenever member, audience, binding or recovery ownership changes.
/// Only a fresh accepted view may enable offers after offline/reconnect.
pub struct CampfireView<'a> {
    pub owner_generation: u64,
    pub revision: u64,
    pub title: &'a str,
    pub location: &'a str,
    pub narration: &'a str,
    pub party_label: &'a str,
    pub notice: &'a str,
    pub connected: bool,
    pub members: &'a [CampfireMember<'a>],
    pub rest: Option<CampfireOffer<'a>>,
    pub continue_action: Option<CampfireOffer<'a>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CampfireAction {
    Rest,
    Continue,
}

/// Selection of an exact current advertised offer; it does not resolve a rest,
/// advance time, restore resources or navigate the authoritative game phase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampfireSelection {
    pub owner_generation: u64,
    pub revision: u64,
    pub action: CampfireAction,
    pub offer_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CampfireValidationError {
    InvalidOwner,
    EmptyText,
    Capacity,
    DuplicateIdentity,
}
impl fmt::Display for CampfireValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidOwner => "campfire presentation owner is invalid",
            Self::EmptyText => "campfire presentation text is empty",
            Self::Capacity => "campfire presentation exceeds bounded capacity",
            Self::DuplicateIdentity => "campfire presentation identities repeat",
        })
    }
}
impl std::error::Error for CampfireValidationError {}

impl CampfireView<'_> {
    pub fn validate(&self) -> Result<(), CampfireValidationError> {
        if self.owner_generation == 0 {
            return Err(CampfireValidationError::InvalidOwner);
        }
        if self.members.len() > 32 {
            return Err(CampfireValidationError::Capacity);
        }
        let mut bytes = 0usize;
        let mut charge = |text: &str| {
            if text.trim().is_empty() {
                return Err(CampfireValidationError::EmptyText);
            }
            bytes = bytes
                .checked_add(text.len())
                .ok_or(CampfireValidationError::Capacity)?;
            if text.len() > 4096 || bytes > 32_768 {
                return Err(CampfireValidationError::Capacity);
            }
            Ok(())
        };
        for text in [
            self.title,
            self.location,
            self.narration,
            self.party_label,
            self.notice,
        ] {
            charge(text)?;
        }
        let mut members = BTreeSet::new();
        for member in self.members {
            if !members.insert(member.key) {
                return Err(CampfireValidationError::DuplicateIdentity);
            }
            for text in [member.key, member.name, member.detail] {
                charge(text)?;
            }
            // A hero with nothing to report has no condition line: it is optional.
            if !member.condition.is_empty() {
                charge(member.condition)?;
            }
        }
        let mut offers = BTreeSet::new();
        for offer in [self.rest.as_ref(), self.continue_action.as_ref()]
            .into_iter()
            .flatten()
        {
            if !offers.insert(offer.id) {
                return Err(CampfireValidationError::DuplicateIdentity);
            }
            charge(offer.id)?;
            charge(offer.label)?;
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::{ActionView, ConceptScene, ControlError, ControlledAction, ThemeToken, UiError};
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
    use web_sys::{Document, Element};

    const STYLES: &str = r#"
.df-campfire{position:relative;isolation:isolate;container-type:inline-size;min-width:0;min-height:560px;padding:28px;background:var(--camp-ink);color:var(--camp-text);font:15px/1.65 system-ui,sans-serif;overflow:hidden;border:1px solid #d9b77a50;border-radius:10px;box-shadow:0 20px 65px #0005}
.df-campfire *{box-sizing:border-box;min-width:0}.df-campfire [hidden]{display:none!important}.df-campfire:before{content:'';position:absolute;inset:0;z-index:-2;background-image:linear-gradient(90deg,#0b100de6,#0b100d45 75%),linear-gradient(0deg,#0b100d 0%,#0b100d15 85%),var(--camp-art);background-size:cover;background-position:center;opacity:.95}
.df-campfire h1,.df-campfire h2,.df-campfire h3,.df-campfire p,.df-campfire button{overflow-wrap:anywhere}.df-campfire .camp-location{color:var(--camp-gold);font:11px/1.6 system-ui,sans-serif;letter-spacing:2px;text-transform:uppercase;margin:0 0 16px}.df-campfire h1{font:400 48px/1.1 Georgia,serif;letter-spacing:-1px;max-width:760px;margin:0 0 20px;text-shadow:0 3px 25px #000}.df-campfire .camp-narration{font:19px/1.7 Georgia,serif;max-width:670px;margin:0 0 36px;color:#eee3ca;text-shadow:0 2px 8px #000}.df-campfire .camp-party-label{font:400 24px/1.3 Georgia,serif;color:#e6cf9d;margin:0 0 16px}.df-campfire .camp-party{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(220px,100%),1fr));gap:12px;margin-bottom:24px}.df-campfire .camp-member{padding:18px;border:1px solid #bca06c60;border-radius:6px;background:linear-gradient(135deg,#252c24ec,#111b15f0);box-shadow:inset 0 2px 0 #d6b57750}.df-campfire .camp-member-name{font:400 22px/1.3 Georgia,serif;margin:0 0 10px;color:#f1dfb9}.df-campfire .camp-condition{display:inline-block;padding:4px 9px;border:1px solid #d9b77a40;border-radius:4px;color:#e7c990;background:#d9b77a0f;font:12px/1.5 system-ui,sans-serif;margin:0 0 10px}.df-campfire .camp-detail{font:13px/1.6 system-ui,sans-serif;color:#d1c7b5;margin:0}.df-campfire .camp-notice{font:12px/1.6 system-ui,sans-serif;color:#ded0b0;padding:12px 14px;margin:0 0 18px;border-left:2px solid var(--camp-gold);background:#151d17e8;max-width:760px}.df-campfire .camp-actions{display:flex;flex-wrap:wrap;gap:12px}.df-campfire .camp-actions button{flex:1 1 180px;max-width:330px;font:600 15px/1.5 system-ui,sans-serif;min-height:48px;padding:12px 18px;border:1px solid #d4b375;border-radius:5px;cursor:pointer;touch-action:manipulation;background:linear-gradient(135deg,#e3c592,#b99151);color:#211b11;box-shadow:inset 0 1px 0 #fff6,0 4px 15px #0003}.df-campfire .camp-actions button:last-child{background:#14221bef;color:#e7d5b0;border-color:#b599654d}.df-campfire .camp-actions button:hover:enabled{border-color:#ffe1a5;filter:brightness(1.12)}.df-campfire .camp-actions button:disabled{background:#222b24;color:#a49c8d;border-color:#897c6259;cursor:default;box-shadow:none}.df-campfire button[aria-busy=true]{border-style:dashed}.df-campfire :focus-visible{outline:3px solid #ffe1a5;outline-offset:4px}
@container(max-width:600px){.df-campfire h1{font-size:36px}.df-campfire .camp-narration{font-size:17px;margin-bottom:26px}.df-campfire .camp-party{grid-template-columns:1fr}.df-campfire .camp-actions button{max-width:none;flex-basis:100%}}
@media(max-width:600px){.df-campfire{padding:20px}.df-campfire h1{font-size:36px}.df-campfire .camp-narration{font-size:17px}.df-campfire .camp-actions button{max-width:none;flex-basis:100%}}
"#;

    #[derive(Debug)]
    pub enum CampfireError {
        InvalidView(CampfireValidationError),
        Dom(UiError),
        Control(ControlError),
        OwnerChanged,
        StaleView,
        Disposed,
    }
    impl fmt::Display for CampfireError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidView(error) => fmt::Display::fmt(error, f),
                Self::Dom(error) => fmt::Display::fmt(error, f),
                Self::Control(error) => fmt::Display::fmt(error, f),
                Self::OwnerChanged => f.write_str("campfire presentation owner changed"),
                Self::StaleView => f.write_str("campfire presentation revision is stale"),
                Self::Disposed => f.write_str("campfire presentation is disposed"),
            }
        }
    }
    impl std::error::Error for CampfireError {}
    impl From<wasm_bindgen::JsValue> for CampfireError {
        fn from(value: wasm_bindgen::JsValue) -> Self {
            Self::Dom(UiError::Browser(value))
        }
    }
    impl From<ControlError> for CampfireError {
        fn from(value: ControlError) -> Self {
            Self::Control(value)
        }
    }
    impl From<CampfireValidationError> for CampfireError {
        fn from(value: CampfireValidationError) -> Self {
            Self::InvalidView(value)
        }
    }
    struct MemberNodes {
        root: Element,
        name: Element,
        condition: Element,
        detail: Element,
    }
    type Select = Box<dyn FnMut(CampfireSelection)>;
    struct Fence {
        active: bool,
        owner: u64,
        revision: u64,
        offers: [Option<String>; 2],
        select: Option<Select>,
    }
    pub struct CampfireSurface {
        document: Document,
        root: Element,
        title: Element,
        location: Element,
        narration: Element,
        party_label: Element,
        notice: Element,
        party: Element,
        members: RefCell<BTreeMap<String, MemberNodes>>,
        controls: [ControlledAction; 2],
        fence: Rc<RefCell<Fence>>,
    }
    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
    ) -> Result<Element, CampfireError> {
        let node = document.create_element(tag)?;
        node.set_class_name(class);
        parent.append_child(&node)?;
        Ok(node)
    }
    fn scrub(node: &web_sys::Node) {
        let mut next = node.first_child();
        while let Some(child) = next {
            next = child.next_sibling();
            scrub(&child);
        }
        node.set_node_value(Some(""));
        node.set_text_content(None);
    }
    fn literal(node: &Element, value: &str) {
        if let Some(text) = node.first_child()
            && text.node_type() == web_sys::Node::TEXT_NODE
            && text.next_sibling().is_none()
        {
            text.set_node_value(Some(value));
        } else {
            scrub(node);
            node.set_text_content(Some(value));
        }
    }
    impl CampfireSurface {
        pub fn create(
            document: &Document,
            view: &CampfireView<'_>,
            select: impl FnMut(CampfireSelection) + 'static,
        ) -> Result<Self, CampfireError> {
            view.validate()?;
            let root = document.create_element("section")?;
            root.set_class_name("df-campfire");
            root.set_attribute(
                "style",
                &format!(
                    "--camp-ink:{};--camp-text:{};--camp-gold:{};--camp-art:url('{}')",
                    ThemeToken::Background.css_value(),
                    ThemeToken::Text.css_value(),
                    ThemeToken::StoryAccent.css_value(),
                    ConceptScene::Campfire.asset_path()
                ),
            )?;
            child(document, &root, "style", "")?.set_text_content(Some(STYLES));
            let location = child(document, &root, "p", "camp-location")?;
            let title = child(document, &root, "h1", "camp-title")?;
            let narration = child(document, &root, "p", "camp-narration")?;
            let party_label = child(document, &root, "h2", "camp-party-label")?;
            let party = child(document, &root, "div", "camp-party")?;
            let notice = child(document, &root, "p", "camp-notice")?;
            notice.set_attribute("role", "status")?;
            let actions = child(document, &root, "div", "camp-actions")?;
            let controls = [
                ControlledAction::create(
                    document,
                    ActionView {
                        label: "Rest",
                        enabled: false,
                        pending: false,
                    },
                )?,
                ControlledAction::create(
                    document,
                    ActionView {
                        label: "Continue",
                        enabled: false,
                        pending: false,
                    },
                )?,
            ];
            let fence = Rc::new(RefCell::new(Fence {
                active: true,
                owner: view.owner_generation,
                revision: view.revision,
                offers: [None, None],
                select: Some(Box::new(select)),
            }));
            for (index, control) in controls.iter().enumerate() {
                actions.append_child(control.element())?;
                let offer_fence = Rc::clone(&fence);
                control.on_activate(move || {
                    let (selection, callback) = {
                        let mut fence = offer_fence.borrow_mut();
                        if !fence.active {
                            return;
                        }
                        let Some(id) = fence.offers.get(index).and_then(Option::as_ref).cloned()
                        else {
                            return;
                        };
                        (
                            CampfireSelection {
                                owner_generation: fence.owner,
                                revision: fence.revision,
                                action: if index == 0 {
                                    CampfireAction::Rest
                                } else {
                                    CampfireAction::Continue
                                },
                                offer_id: id,
                            },
                            fence.select.take(),
                        )
                    };
                    if let Some(mut callback) = callback {
                        callback(selection);
                        let mut fence = offer_fence.borrow_mut();
                        if fence.active {
                            fence.select = Some(callback);
                        }
                    }
                })?;
            }
            let surface = Self {
                document: document.clone(),
                root,
                title,
                location,
                narration,
                party_label,
                notice,
                party,
                members: RefCell::new(BTreeMap::new()),
                controls,
                fence,
            };
            surface.update(view)?;
            Ok(surface)
        }
        pub fn root(&self) -> &Element {
            &self.root
        }
        pub fn update(&self, view: &CampfireView<'_>) -> Result<(), CampfireError> {
            {
                let fence = self.fence.borrow();
                if !fence.active {
                    return Err(CampfireError::Disposed);
                }
                if fence.owner != view.owner_generation {
                    drop(fence);
                    self.dispose()?;
                    return Err(CampfireError::OwnerChanged);
                }
                if view.revision < fence.revision {
                    return Err(CampfireError::StaleView);
                }
            }
            self.fence.borrow_mut().offers = [None, None];
            if let Err(error) = view.validate() {
                self.dispose()?;
                return Err(error.into());
            }
            if let Err(error) = self.render(view) {
                self.dispose()?;
                return Err(error);
            }
            let mut fence = self.fence.borrow_mut();
            fence.revision = view.revision;
            for (index, offer) in [view.rest.as_ref(), view.continue_action.as_ref()]
                .into_iter()
                .enumerate()
            {
                if let Some(offer) =
                    offer.filter(|offer| view.connected && offer.enabled && !offer.pending)
                {
                    fence.offers[index] = Some(offer.id.to_owned());
                }
            }
            Ok(())
        }
        fn render(&self, view: &CampfireView<'_>) -> Result<(), CampfireError> {
            for (node, text) in [
                (&self.title, view.title),
                (&self.location, view.location),
                (&self.narration, view.narration),
                (&self.party_label, view.party_label),
                (&self.notice, view.notice),
            ] {
                literal(node, text);
            }
            let mut mounted = self.members.borrow_mut();
            let obsolete: Vec<_> = mounted
                .keys()
                .filter(|key| !view.members.iter().any(|member| member.key == key.as_str()))
                .cloned()
                .collect();
            for key in obsolete {
                if let Some(member) = mounted.remove(&key) {
                    scrub(&member.root);
                    member.root.remove();
                }
            }
            for member in view.members {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    mounted.entry(member.key.to_owned())
                {
                    let root = child(&self.document, &self.party, "article", "camp-member")?;
                    let name = child(&self.document, &root, "h3", "camp-member-name")?;
                    let condition = child(&self.document, &root, "p", "camp-condition")?;
                    let detail = child(&self.document, &root, "p", "camp-detail")?;
                    entry.insert(MemberNodes {
                        root,
                        name,
                        condition,
                        detail,
                    });
                }
                if let Some(nodes) = mounted.get(member.key) {
                    literal(&nodes.name, member.name);
                    literal(&nodes.condition, member.condition);
                    if member.condition.is_empty() {
                        nodes.condition.set_attribute("hidden", "")?;
                    } else {
                        nodes.condition.remove_attribute("hidden")?;
                    }
                    literal(&nodes.detail, member.detail);
                    self.party.append_child(&nodes.root)?;
                }
            }
            for (control, offer) in self
                .controls
                .iter()
                .zip([view.rest.as_ref(), view.continue_action.as_ref()])
            {
                if let Some(offer) = offer {
                    control.element().remove_attribute("hidden")?;
                    scrub(control.element());
                    control.update(ActionView {
                        label: offer.label,
                        enabled: view.connected && offer.enabled,
                        pending: offer.pending,
                    })?;
                } else {
                    control.element().set_disabled(true);
                    control.element().set_attribute("hidden", "")?;
                    scrub(control.element());
                }
            }
            Ok(())
        }
        /// Immediate offer fencing for an offline or hidden role. Re-enabling
        /// requires update with a current accepted view, rather than old offers.
        pub fn suspend(&self) {
            self.fence.borrow_mut().offers = [None, None];
            for control in &self.controls {
                control.element().set_disabled(true);
            }
        }
        pub fn dispose(&self) -> Result<(), CampfireError> {
            {
                let mut fence = self.fence.borrow_mut();
                fence.active = false;
                fence.offers = [None, None];
                fence.select = None;
            }
            // Scrub owned leaves and keyed cards even if another scope detached
            // their DOM nodes. Retained references must not keep private content.
            for member in self.members.borrow().values() {
                scrub(&member.root);
            }
            self.members.borrow_mut().clear();
            for node in [
                &self.title,
                &self.location,
                &self.narration,
                &self.party_label,
                &self.notice,
            ] {
                scrub(node);
            }
            scrub(&self.root);
            self.root.remove();
            let mut failure = None;
            for control in &self.controls {
                scrub(control.element());
                if let Err(error) = control.dispose()
                    && failure.is_none()
                {
                    failure = Some(error);
                }
            }
            if let Some(error) = failure {
                return Err(error.into());
            }
            Ok(())
        }
    }
    impl Drop for CampfireSurface {
        fn drop(&mut self) {
            if self.dispose().is_err() {
                self.root.remove();
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::{CampfireError, CampfireSurface};

#[cfg(test)]
mod tests {
    use super::*;
    fn view<'a>(members: &'a [CampfireMember<'a>]) -> CampfireView<'a> {
        CampfireView {
            owner_generation: 1,
            revision: 7,
            title: "Under the stars",
            location: "The old road",
            narration: "The embers settle as the wind quiets.",
            party_label: "Around the fire",
            notice: "Awaiting your next choice",
            connected: true,
            members,
            rest: None,
            continue_action: None,
        }
    }
    #[test]
    fn refuses_unknown_owner_and_duplicate_member_keys() {
        let members = [
            CampfireMember {
                key: "a",
                name: "A",
                condition: "Tired",
                detail: "Supplied state",
            },
            CampfireMember {
                key: "a",
                name: "B",
                condition: "Alert",
                detail: "Supplied state",
            },
        ];
        assert_eq!(
            view(&members).validate(),
            Err(CampfireValidationError::DuplicateIdentity)
        );
        let mut current = view(&[]);
        current.owner_generation = 0;
        assert_eq!(
            current.validate(),
            Err(CampfireValidationError::InvalidOwner)
        );
    }
    #[test]
    fn a_member_with_no_condition_is_valid_and_blank_text_elsewhere_is_not() {
        let healthy = [CampfireMember {
            key: "a",
            name: "Brakka",
            condition: "",
            detail: "HP 13 / 13",
        }];
        assert_eq!(view(&healthy).validate(), Ok(()));
        let nameless = [CampfireMember {
            key: "a",
            name: "",
            condition: "",
            detail: "HP 13 / 13",
        }];
        assert_eq!(
            view(&nameless).validate(),
            Err(CampfireValidationError::EmptyText)
        );
        let blank_condition = [CampfireMember {
            key: "a",
            name: "Brakka",
            condition: "  ",
            detail: "HP 13 / 13",
        }];
        assert_eq!(
            view(&blank_condition).validate(),
            Err(CampfireValidationError::EmptyText)
        );
    }
    #[test]
    fn refuses_duplicate_offer_ids_without_interpreting_condition_text() {
        let mut current = view(&[]);
        current.rest = Some(CampfireOffer {
            id: "same",
            label: "Rest",
            enabled: true,
            pending: false,
        });
        current.continue_action = Some(CampfireOffer {
            id: "same",
            label: "Continue",
            enabled: true,
            pending: false,
        });
        assert_eq!(
            current.validate(),
            Err(CampfireValidationError::DuplicateIdentity)
        );
        current.continue_action = None;
        current.narration = "<img src=x onerror=alert(1)>";
        assert_eq!(current.validate(), Ok(()));
    }
    #[test]
    fn refuses_empty_and_excessive_text_before_mounting() {
        let mut current = view(&[]);
        current.notice = " ";
        assert_eq!(current.validate(), Err(CampfireValidationError::EmptyText));
        let excessive = "x".repeat(4097);
        current.notice = &excessive;
        assert_eq!(current.validate(), Err(CampfireValidationError::Capacity));
    }
}
