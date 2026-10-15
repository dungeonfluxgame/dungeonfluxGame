//! C01 compact creation surface. The mounting owner supplies server offers and identity defaults.
//! The surface owns its listeners, so the gameplay renderer's clear_callbacks cannot kill them.

use std::{cell::Cell, rc::Rc};

use df_protocol::common as rpc;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Document, Element, HtmlButtonElement, HtmlSelectElement, HtmlTextAreaElement};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IdentityDraft {
    pub(super) appearance: Vec<rpc::JourneyChoice>,
    pub(super) origin: String,
    pub(super) occupation: String,
    pub(super) motivation: String,
    pub(super) backstory: String,
    pub(super) facts: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HeroDraft {
    pub(super) name: String,
    pub(super) choices: Vec<rpc::JourneyChoice>,
    pub(super) identity: IdentityDraft,
}

impl IdentityDraft {
    pub(super) fn from_rpc(identity: &rpc::HeroIdentity) -> Result<Self, JsValue> {
        let look = identity
            .appearance
            .as_ref()
            .ok_or_else(|| JsValue::from_str("Hero details are incomplete."))?;
        Ok(Self {
            appearance: [
                ("presentation", &look.presentation),
                ("pronouns", &look.pronouns),
                ("age", &look.age),
                ("build", &look.build),
                ("skin", &look.skin),
                ("hair", &look.hair),
                ("hair-color", &look.hair_color),
                ("eyes", &look.eyes),
                ("mark", &look.mark),
                ("attire", &look.attire),
            ]
            .into_iter()
            .map(|(group, value)| rpc::JourneyChoice {
                group_id: group.to_owned(),
                option_id: value.clone(),
            })
            .collect(),
            origin: identity.origin.clone(),
            occupation: identity.occupation.clone(),
            motivation: identity.motivation.clone(),
            backstory: identity.backstory.clone(),
            facts: identity.facts.clone(),
        })
    }

    pub(super) fn to_rpc(&self) -> Result<rpc::HeroIdentity, JsValue> {
        let field = |id: &str| -> Result<String, JsValue> {
            let mut fields = self
                .appearance
                .iter()
                .filter(|choice| choice.group_id == id);
            let choice = fields
                .next()
                .ok_or_else(|| JsValue::from_str("Choose all your hero's details."))?;
            if fields.next().is_some() || choice.option_id.is_empty() {
                return Err(JsValue::from_str("Choose one option for each hero detail."));
            }
            Ok(choice.option_id.clone())
        };
        Ok(rpc::HeroIdentity {
            appearance: Some(rpc::HeroAppearance {
                presentation: field("presentation")?,
                pronouns: field("pronouns")?,
                age: field("age")?,
                build: field("build")?,
                skin: field("skin")?,
                hair: field("hair")?,
                hair_color: field("hair-color")?,
                eyes: field("eyes")?,
                mark: field("mark")?,
                attire: field("attire")?,
            }),
            origin: self.origin.clone(),
            occupation: self.occupation.clone(),
            motivation: self.motivation.clone(),
            backstory: self.backstory.clone(),
            facts: self.facts.clone(),
        })
    }
}

impl HeroDraft {
    pub(super) fn selection(&self) -> Result<rpc::CharacterSelection, JsValue> {
        Ok(rpc::CharacterSelection {
            name: self.name.clone(),
            choices: self.choices.clone(),
            identity: Some(self.identity.to_rpc()?),
        })
    }
}

pub(super) enum CreationEvent {
    Preview {
        draft_version: u64,
        appearance_text: String,
        backstory_text: String,
        current: HeroDraft,
    },
    Confirm(HeroDraft),
}

struct Listener {
    element: Element,
    event: &'static str,
    callback: Closure<dyn FnMut(web_sys::Event)>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.element.remove_event_listener_with_callback(
            self.event,
            self.callback.as_ref().unchecked_ref(),
        );
    }
}

#[derive(Clone)]
struct StoryControls {
    origin: HtmlTextAreaElement,
    occupation: HtmlTextAreaElement,
    motivation: HtmlTextAreaElement,
}

#[derive(Clone)]
struct Controls {
    name: HtmlTextAreaElement,
    choices: Vec<(rpc::JourneyGroup, HtmlSelectElement)>,
    appearance: Vec<(rpc::JourneyGroup, HtmlSelectElement)>,
    story: StoryControls,
    story_menus: Vec<(rpc::JourneyGroup, HtmlSelectElement)>,
    facts: Vec<HtmlTextAreaElement>,
    paragraph: HtmlTextAreaElement,
    paragraph_edited: Rc<Cell<bool>>,
    appearance_text: HtmlTextAreaElement,
    backstory_text: HtmlTextAreaElement,
    missing: Element,
    chips: Element,
    confirm: HtmlButtonElement,
    version: Rc<Cell<u64>>,
    pending: Rc<Cell<bool>>,
    text_reviewed: Rc<Cell<bool>>,
}

fn choices(controls: &[(rpc::JourneyGroup, HtmlSelectElement)]) -> Vec<rpc::JourneyChoice> {
    controls
        .iter()
        .map(|(group, select)| rpc::JourneyChoice {
            group_id: group.group_id.clone(),
            option_id: select.value(),
        })
        .collect()
}

fn missing_choices(
    name: &str,
    groups: &[rpc::JourneyGroup],
    choices: &[rpc::JourneyChoice],
) -> Vec<String> {
    let mut missing = Vec::new();
    if name.trim().is_empty() {
        missing.push("Name your hero".to_owned());
    } else if name.len() > 24
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || matches!(byte, b'-' | b'_'))
    {
        missing.push("Use up to 24 letters, dashes or underscores for your hero's name".to_owned());
    }
    for group in groups {
        let choice = choices
            .iter()
            .find(|choice| choice.group_id == group.group_id);
        let valid = choice
            .and_then(|choice| {
                group.options.iter().find(|option| {
                    option.option_id == choice.option_id
                        && (option.requires_group.is_empty()
                            || choices.iter().any(|required| {
                                required.group_id == option.requires_group
                                    && option.requires_options.contains(&required.option_id)
                            }))
                })
            })
            .is_some();
        if !valid {
            missing.push(format!("Choose {}", group.label));
        }
    }
    missing
}

impl Controls {
    fn draft(&self) -> HeroDraft {
        HeroDraft {
            name: self.name.value(),
            choices: choices(&self.choices),
            identity: IdentityDraft {
                appearance: choices(&self.appearance),
                origin: self.story.origin.value(),
                occupation: self.story.occupation.value(),
                motivation: self.story.motivation.value(),
                backstory: self.paragraph.value(),
                facts: self
                    .facts
                    .iter()
                    .map(HtmlTextAreaElement::value)
                    .filter(|fact| !fact.trim().is_empty())
                    .collect(),
            },
        }
    }

    fn update_missing(&self) -> bool {
        let draft = self.draft();
        let groups: Vec<_> = self
            .choices
            .iter()
            .map(|(group, _)| group.clone())
            .collect();
        let mut missing = missing_choices(&draft.name, &groups, &draft.choices);
        if !self.text_reviewed.get()
            && (!self.appearance_text.value().trim().is_empty()
                || !self.backstory_text.value().trim().is_empty())
        {
            missing.push("Press Read my description and review the details".to_owned());
        }
        let groups: Vec<_> = self
            .appearance
            .iter()
            .map(|(group, _)| group.clone())
            .collect();
        missing.extend(missing_choices("hero", &groups, &draft.identity.appearance));
        for (label, value) in [
            ("where you are from", draft.identity.origin.as_str()),
            ("what you did", draft.identity.occupation.as_str()),
            ("what you want or fear", draft.identity.motivation.as_str()),
            ("your backstory", draft.identity.backstory.as_str()),
        ] {
            if value.trim().is_empty() {
                missing.push(format!("Add {label}"));
            }
        }
        if draft.identity.facts.len() < 3 {
            missing.push("Add the three story facts".to_owned());
        }
        let ready = missing.is_empty();
        let chips = self
            .appearance
            .iter()
            .filter_map(|(group, select)| {
                let selected = select.value();
                group
                    .options
                    .iter()
                    .find(|option| option.option_id == selected)
                    .map(|option| option.label.as_str())
            })
            .collect::<Vec<_>>()
            .join(" · ");
        self.chips.set_text_content(Some(&chips));
        self.missing.set_text_content(Some(if self.pending.get() {
            "Confirming your hero…"
        } else if ready {
            "Your hero is ready to confirm."
        } else {
            "Still needed:"
        }));
        if !ready && !self.pending.get() {
            self.missing
                .set_text_content(Some(&format!("Still needed: {}.", missing.join("; "))));
        }
        self.confirm.set_disabled(!ready || self.pending.get());
        ready && !self.pending.get()
    }

    fn edited(&self) {
        self.version.set(self.version.get().saturating_add(1));
        self.update_missing();
    }
}

pub(super) struct HeroCreator {
    root: Element,
    controls: Controls,
    listeners: Vec<Listener>,
    feedback: Element,
}

fn node(
    document: &Document,
    parent: &Element,
    tag: &str,
    class: &str,
    text: &str,
) -> Result<Element, JsValue> {
    let element = document.create_element(tag)?;
    element.set_class_name(class);
    element.set_text_content(Some(text));
    parent.append_child(&element)?;
    Ok(element)
}

fn text_input(
    document: &Document,
    parent: &Element,
    label: &str,
    value: &str,
    maximum: u32,
) -> Result<HtmlTextAreaElement, JsValue> {
    let label = node(document, parent, "label", "hero-field", label)?;
    let input = node(document, &label, "textarea", "", "")?.dyn_into::<HtmlTextAreaElement>()?;
    input.set_rows(1);
    input.set_max_length(
        i32::try_from(maximum).map_err(|_| JsValue::from_str("text limit invalid"))?,
    );
    input.set_value(value);
    Ok(input)
}

fn select(
    document: &Document,
    parent: &Element,
    group: &rpc::JourneyGroup,
    selected: Option<&str>,
) -> Result<HtmlSelectElement, JsValue> {
    let label = node(document, parent, "label", "hero-field", &group.label)?;
    let select = node(document, &label, "select", "", "")?.dyn_into::<HtmlSelectElement>()?;
    let placeholder = node(document, &select, "option", "", "Choose…")?;
    placeholder.set_attribute("value", "")?;
    for option in &group.options {
        let item = node(document, &select, "option", "", &option.label)?;
        item.set_attribute("value", &option.option_id)?;
    }
    if let Some(id) = selected {
        select.set_value(id);
    } else if let [only] = group.options.as_slice() {
        select.set_value(&only.option_id);
    }
    if let [only] = group.options.as_slice() {
        select.set_attribute("hidden", "")?;
        label.set_class_name("hero-fixed-field");
        node(document, &label, "span", "", &only.label)?;
    }
    Ok(select)
}

impl HeroCreator {
    /// No submitted identity is inferred in the browser. `initial` and menus come from server.
    pub(super) fn create(
        document: &Document,
        parent: &Element,
        offer: &rpc::CharacterCreationOffer,
        identity_groups: &[rpc::JourneyGroup],
        initial: &IdentityDraft,
        on_event: impl Fn(CreationEvent) + 'static,
    ) -> Result<Self, JsValue> {
        if !(3..=6).contains(&initial.facts.len()) {
            return Err(JsValue::from_str(
                "creation needs three to six reviewed facts",
            ));
        }
        let root = node(document, parent, "section", "hero-creator", "")?;
        node(
            document,
            &root,
            "style",
            "",
            include_str!("hero_creator.css"),
        )?;
        node(document, &root, "h1", "", "Make your hero")?;
        node(
            document,
            &root,
            "p",
            "hero-intro",
            "Choose your look and past, or describe them in your own words. Review the details before confirming.",
        )?;
        let name = text_input(document, &root, "Name your hero", "", 24)?;
        let basics = node(document, &root, "div", "hero-basics", "")?;
        let advanced = node(document, &root, "details", "hero-details", "")?;
        node(
            document,
            &advanced,
            "summary",
            "",
            "Skills, languages and equipment",
        )?;
        let mut build = Vec::new();
        for group in &offer.groups {
            let parent = if matches!(
                group.group_id.as_str(),
                "species" | "class" | "background" | "build-package"
            ) {
                &basics
            } else {
                &advanced
            };
            let input = select(document, parent, group, None)?;
            build.push((group.clone(), input));
        }
        let chips = node(document, &root, "p", "hero-chips", "")?;
        let look = node(document, &root, "details", "hero-details", "")?;
        node(
            document,
            &look,
            "summary",
            "",
            "Your look · correct the details",
        )?;
        let appearance_text =
            text_input(document, &look, "Describe your look (optional)", "", 1000)?;
        appearance_text
            .set_placeholder("A wiry grey-haired dwarf woman with a burn scar and a blue cloak");
        let grid = node(document, &look, "div", "hero-look-grid", "")?;
        let mut appearance = Vec::new();
        for group in identity_groups.iter().filter(|group| {
            !matches!(
                group.group_id.as_str(),
                "origin" | "occupation" | "motivation"
            )
        }) {
            let selected = initial
                .appearance
                .iter()
                .find(|choice| choice.group_id == group.group_id)
                .map(|choice| choice.option_id.as_str());
            appearance.push((group.clone(), select(document, &grid, group, selected)?));
        }
        let past = node(document, &root, "details", "hero-details", "")?;
        node(
            document,
            &past,
            "summary",
            "",
            "Your past · three facts and your story",
        )?;
        node(
            document,
            &past,
            "p",
            "hero-help",
            "These details are shared with the table and may be used in the story. They grant no skills or equipment.",
        )?;
        let backstory_text = text_input(document, &past, "Tell your story (optional)", "", 4000)?;
        let mut story_menus = Vec::new();
        for group in identity_groups.iter().filter(|group| {
            matches!(
                group.group_id.as_str(),
                "origin" | "occupation" | "motivation"
            )
        }) {
            let value = match group.group_id.as_str() {
                "origin" => &initial.origin,
                "occupation" => &initial.occupation,
                _ => &initial.motivation,
            };
            let selected = group
                .options
                .iter()
                .find(|option| &option.description == value)
                .map(|option| option.option_id.as_str());
            story_menus.push((group.clone(), select(document, &past, group, selected)?));
        }
        let story = StoryControls {
            origin: text_input(
                document,
                &past,
                "Where are you from? · edit",
                &initial.origin,
                160,
            )?,
            occupation: text_input(
                document,
                &past,
                "What did you do? · edit",
                &initial.occupation,
                160,
            )?,
            motivation: text_input(
                document,
                &past,
                "What do you want or fear? · edit",
                &initial.motivation,
                160,
            )?,
        };
        let paragraph = text_input(
            document,
            &past,
            "Your backstory · edit it freely",
            &initial.backstory,
            480,
        )?;
        paragraph.set_rows(3);
        let mut facts = Vec::new();
        for index in 0..6 {
            let fact = initial.facts.get(index).map_or("", String::as_str);
            let input = text_input(
                document,
                &past,
                &format!("Story fact {}", index + 1),
                fact,
                160,
            )?;
            if index >= 3
                && fact.is_empty()
                && let Some(label) = input.parent_element()
            {
                label.set_attribute("hidden", "")?;
            }
            facts.push(input);
        }
        let read = node(
            document,
            &root,
            "button",
            "hero-read",
            "Read my description",
        )?;
        read.set_attribute("type", "button")?;
        let feedback = node(document, &root, "p", "hero-feedback", "")?;
        feedback.set_attribute("role", "status")?;
        let footer = node(document, &root, "div", "hero-confirm", "")?;
        node(
            document,
            &footer,
            "p",
            "hero-help",
            "Your look and story will be shared at the table.",
        )?;
        let missing = node(document, &footer, "p", "hero-missing", "")?;
        missing.set_attribute("aria-live", "polite")?;
        let confirm = node(document, &footer, "button", "", "Confirm your hero")?
            .dyn_into::<HtmlButtonElement>()?;
        confirm.set_type("button");
        let controls = Controls {
            name,
            choices: build,
            appearance,
            story,
            story_menus: story_menus.clone(),
            facts,
            paragraph,
            paragraph_edited: Rc::new(Cell::new(false)),
            appearance_text,
            backstory_text,
            missing,
            chips,
            confirm,
            version: Rc::new(Cell::new(0)),
            pending: Rc::new(Cell::new(false)),
            text_reviewed: Rc::new(Cell::new(true)),
        };
        let on_event = Rc::new(on_event);
        let mut surface = Self {
            root,
            controls,
            listeners: Vec::new(),
            feedback,
        };
        // Cards expose only the server's offered species/background choices, including their
        // descriptions. The underlying select carries the same submitted option id.
        for (group, select) in surface.controls.choices.clone() {
            if !matches!(group.group_id.as_str(), "species" | "background") {
                continue;
            }
            let Some(label) = select.parent_element() else {
                continue;
            };
            label.set_class_name("hero-field");
            if let Some(fixed_label) = label.query_selector(":scope > span")? {
                fixed_label.remove();
            }
            select.set_attribute("hidden", "")?;
            let cards = node(document, &label, "div", "hero-cards", "")?;
            for option in &group.options {
                let card = node(document, &cards, "button", "hero-card", "")?;
                card.set_attribute("type", "button")?;
                card.set_attribute("data-option-id", &option.option_id)?;
                card.set_attribute(
                    "aria-pressed",
                    if select.value() == option.option_id {
                        "true"
                    } else {
                        "false"
                    },
                )?;
                node(document, &card, "strong", "", &option.label)?;
                node(document, &card, "span", "", &option.description)?;
                let selected = select.clone();
                let id = option.option_id.clone();
                let siblings = cards.clone();
                let controls = surface.controls.clone();
                surface.listen(card, "click", move |_| {
                    selected.set_value(&id);
                    controls.edited();
                    if let Ok(buttons) = siblings.query_selector_all("button") {
                        for index in 0..buttons.length() {
                            if let Some(button) = buttons.item(index) {
                                let button = button.unchecked_into::<Element>();
                                let pressed =
                                    button.get_attribute("data-option-id").as_deref() == Some(&id);
                                let _ = button.set_attribute(
                                    "aria-pressed",
                                    if pressed { "true" } else { "false" },
                                );
                            }
                        }
                    }
                })?;
            }
        }
        let inputs = surface.root.query_selector_all("textarea, select")?;
        for index in 0..inputs.length() {
            let Some(input) = inputs.item(index) else {
                continue;
            };
            let element = input.dyn_into::<Element>()?;
            let controls = surface.controls.clone();
            surface.listen(element, "input", move |_| controls.edited())?;
        }
        for input in [
            surface.controls.appearance_text.clone(),
            surface.controls.backstory_text.clone(),
        ] {
            let controls = surface.controls.clone();
            surface.listen(input.unchecked_into(), "input", move |_| {
                controls.text_reviewed.set(false);
                controls.update_missing();
            })?;
        }
        let edited = Rc::clone(&surface.controls.paragraph_edited);
        surface.listen(
            surface.controls.paragraph.clone().unchecked_into(),
            "input",
            move |_| edited.set(true),
        )?;
        for (group, select) in story_menus {
            let controls = surface.controls.clone();
            surface.listen(select.clone().unchecked_into(), "input", move |_| {
                let id = select.value();
                let Some(option) = group.options.iter().find(|option| option.option_id == id)
                else {
                    return;
                };
                let (input, fact_index) = match group.group_id.as_str() {
                    "origin" => (&controls.story.origin, 0),
                    "occupation" => (&controls.story.occupation, 1),
                    "motivation" => (&controls.story.motivation, 2),
                    _ => return,
                };
                input.set_value(&option.description);
                if let Some(fact) = controls.facts.get(fact_index) {
                    fact.set_value(&option.description);
                }
                if !controls.paragraph_edited.get() {
                    controls.paragraph.set_value(&format!(
                        "{}. {}. {}.",
                        controls.story.origin.value(),
                        controls.story.occupation.value(),
                        controls.story.motivation.value()
                    ));
                }
                controls.edited();
            })?;
        }
        let controls = surface.controls.clone();
        let event = Rc::clone(&on_event);
        surface.listen(read, "click", move |_| {
            let appearance_text = controls.appearance_text.value();
            let backstory_text = controls.backstory_text.value();
            if appearance_text.trim().is_empty() && backstory_text.trim().is_empty() {
                return;
            }
            event(CreationEvent::Preview {
                draft_version: controls.version.get(),
                appearance_text,
                backstory_text,
                current: controls.draft(),
            });
        })?;
        let controls = surface.controls.clone();
        surface.listen(
            controls.confirm.clone().unchecked_into(),
            "click",
            move |_| {
                if controls.update_missing() {
                    controls.pending.set(true);
                    controls.update_missing();
                    on_event(CreationEvent::Confirm(controls.draft()));
                }
            },
        )?;
        surface.controls.update_missing();
        Ok(surface)
    }

    fn listen(
        &mut self,
        element: Element,
        event: &'static str,
        callback: impl FnMut(web_sys::Event) + 'static,
    ) -> Result<(), JsValue> {
        let callback = Closure::wrap(Box::new(callback) as Box<dyn FnMut(web_sys::Event)>);
        element.add_event_listener_with_callback(event, callback.as_ref().unchecked_ref())?;
        self.listeners.push(Listener {
            element,
            event,
            callback,
        });
        Ok(())
    }

    pub(super) fn root(&self) -> &Element {
        &self.root
    }

    #[cfg(test)]
    pub(super) fn draft(&self) -> HeroDraft {
        self.controls.draft()
    }

    pub(super) fn dispose(self) -> Result<(), JsValue> {
        self.root.remove();
        // Dropping the surface releases the listeners it owns, independently of page redraws.
        Ok(())
    }

    /// The caller clears pending only after a durable outcome or a preparation failure.
    pub(super) fn set_pending(&self, pending: bool) {
        self.controls.pending.set(pending);
        self.controls.update_missing();
    }

    /// Stale previews cannot overwrite typing or menu corrections made after dispatch.
    pub(super) fn apply_preview(
        &self,
        version: u64,
        identity: &IdentityDraft,
        warnings: &[String],
    ) -> bool {
        if self.controls.pending.get()
            || self.controls.version.get() != version
            || !(3..=6).contains(&identity.facts.len())
        {
            return false;
        }
        for (group, select) in &self.controls.appearance {
            if let Some(choice) = identity
                .appearance
                .iter()
                .find(|choice| choice.group_id == group.group_id)
            {
                select.set_value(&choice.option_id);
            }
        }
        self.controls.story.origin.set_value(&identity.origin);
        self.controls
            .story
            .occupation
            .set_value(&identity.occupation);
        self.controls
            .story
            .motivation
            .set_value(&identity.motivation);
        for (group, select) in &self.controls.story_menus {
            let text = match group.group_id.as_str() {
                "origin" => &identity.origin,
                "occupation" => &identity.occupation,
                _ => &identity.motivation,
            };
            let option = group
                .options
                .iter()
                .find(|option| &option.description == text);
            select.set_value(option.map_or("", |option| option.option_id.as_str()));
        }
        for (index, input) in self.controls.facts.iter().enumerate() {
            let fact = identity.facts.get(index).map_or("", String::as_str);
            input.set_value(fact);
            if let Some(label) = input.parent_element() {
                if index >= 3 && fact.is_empty() {
                    let _ = label.set_attribute("hidden", "");
                } else {
                    let _ = label.remove_attribute("hidden");
                }
            }
        }
        self.controls.paragraph.set_value(&identity.backstory);
        self.controls.paragraph_edited.set(true);
        self.feedback.set_text_content(Some(&warnings.join(" ")));
        self.controls.text_reviewed.set(true);
        self.controls.edited();
        true
    }

    pub(super) fn show_error(&self, message: &str) {
        self.feedback.set_text_content(Some(message));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrected_identity_is_included_in_the_confirmed_character_selection() {
        let identity = rpc::HeroIdentity {
            appearance: Some(rpc::HeroAppearance {
                presentation: "feminine".to_owned(),
                pronouns: "she-her".to_owned(),
                age: "elder".to_owned(),
                build: "wiry".to_owned(),
                skin: "brown".to_owned(),
                hair: "braids".to_owned(),
                hair_color: "grey".to_owned(),
                eyes: "brown".to_owned(),
                mark: "burn-scar".to_owned(),
                attire: "blue-cloak".to_owned(),
            }),
            origin: "From the highlands".to_owned(),
            occupation: "A guard".to_owned(),
            motivation: "Protect home".to_owned(),
            backstory: "A highland guard who wants to protect home.".to_owned(),
            facts: vec![
                "From the highlands".to_owned(),
                "A guard".to_owned(),
                "Protect home".to_owned(),
            ],
        };
        let draft = IdentityDraft::from_rpc(&identity).unwrap();
        assert_eq!(draft.to_rpc().unwrap(), identity);
        let hero = HeroDraft {
            name: "Maren".to_owned(),
            choices: vec![],
            identity: draft.clone(),
        };
        assert_eq!(hero.selection().unwrap().identity, Some(identity));
        let mut incomplete = draft;
        incomplete.appearance.clear();
        assert!(incomplete.to_rpc().is_err());
    }

    #[test]
    fn missing_choices_names_each_unmade_selection_before_confirmation() {
        let group = rpc::JourneyGroup {
            group_id: "species".to_owned(),
            label: "Species".to_owned(),
            options: vec![rpc::JourneyOption {
                option_id: "dwarf".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            missing_choices("", std::slice::from_ref(&group), &[]),
            ["Name your hero", "Choose Species"]
        );
        assert!(
            missing_choices(
                "Maren",
                &[group],
                &[rpc::JourneyChoice {
                    group_id: "species".to_owned(),
                    option_id: "dwarf".to_owned(),
                }]
            )
            .is_empty()
        );
    }

    #[test]
    fn invalid_names_are_explained_before_confirmation() {
        for name in [
            "Maren of the Coast",
            "123",
            "Mären",
            "abcdefghijklmnopqrstuvwxy",
        ] {
            assert_eq!(
                missing_choices(name, &[], &[]),
                ["Use up to 24 letters, dashes or underscores for your hero's name"]
            );
        }
        assert!(missing_choices("Maren-Coast", &[], &[]).is_empty());
    }

    #[test]
    fn compact_surface_retains_listeners_and_rejects_a_preview_after_typing() {
        let document = web_sys::window().unwrap().document().unwrap();
        let parent = document.create_element("div").unwrap();
        let group = rpc::JourneyGroup {
            group_id: "species".to_owned(),
            label: "Species".to_owned(),
            options: vec![rpc::JourneyOption {
                option_id: "dwarf".to_owned(),
                label: "Dwarf".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let initial = IdentityDraft {
            appearance: vec![],
            origin: "From the highlands".to_owned(),
            occupation: "A guard".to_owned(),
            motivation: "Protect home".to_owned(),
            backstory: "From the highlands, a guard who wants to protect home.".to_owned(),
            facts: vec![
                "From the highlands".to_owned(),
                "A guard".to_owned(),
                "Protect home".to_owned(),
            ],
        };
        let surface = HeroCreator::create(
            &document,
            &parent,
            &rpc::CharacterCreationOffer {
                groups: vec![group],
                ..Default::default()
            },
            &[],
            &initial,
            |event| match event {
                CreationEvent::Preview {
                    draft_version,
                    appearance_text,
                    backstory_text,
                    current,
                } => {
                    assert!(!appearance_text.is_empty() || !backstory_text.is_empty());
                    let _ = (draft_version, current);
                }
                CreationEvent::Confirm(draft) => assert!(!draft.name.is_empty()),
            },
        )
        .unwrap();
        assert_eq!(
            surface
                .root()
                .query_selector_all("details")
                .unwrap()
                .length(),
            3
        );
        assert!(surface.controls.confirm.disabled());
        surface.controls.name.set_value("Maren");
        surface
            .controls
            .name
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        assert!(!surface.controls.confirm.disabled());
        assert!(!surface.apply_preview(0, &initial, &[]));
        assert_eq!(surface.draft().name, "Maren");
        assert!(surface.apply_preview(1, &initial, &["Review your details".to_owned()]));
        surface.set_pending(true);
        assert!(surface.controls.confirm.disabled());
        assert!(!surface.apply_preview(2, &initial, &[]));
        surface.set_pending(false);
        assert!(!surface.controls.confirm.disabled());
        surface.show_error("Choose again");
        assert_eq!(
            surface.feedback.text_content().as_deref(),
            Some("Choose again")
        );
        assert!(!surface.listeners.is_empty());
        let card = surface
            .root()
            .query_selector(".hero-card")
            .unwrap()
            .unwrap();
        assert_eq!(card.get_attribute("aria-pressed").as_deref(), Some("true"));
        card.dispatch_event(&web_sys::Event::new("click").unwrap())
            .unwrap();
        assert_eq!(surface.draft().choices[0].option_id, "dwarf");

        surface
            .controls
            .appearance_text
            .set_value("A grey-haired woman");
        surface
            .controls
            .appearance_text
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        assert!(surface.controls.confirm.disabled());
        assert!(
            surface
                .controls
                .missing
                .text_content()
                .unwrap()
                .contains("Read my description")
        );
        let version = surface.controls.version.get();
        assert!(surface.apply_preview(version, &initial, &[]));
        assert!(!surface.controls.confirm.disabled());
        let root = surface.root().clone();
        surface.dispose().unwrap();
        assert!(root.parent_element().is_none());
    }
}
