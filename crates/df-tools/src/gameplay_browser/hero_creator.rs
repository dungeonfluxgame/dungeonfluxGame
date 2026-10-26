//! C01 compact creation surface. The mounting owner supplies server offers and identity defaults.
//! The surface owns its listeners, so the gameplay renderer's clear_callbacks cannot kill them.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use df_protocol::common as rpc;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Document, Element, HtmlButtonElement, HtmlSelectElement, HtmlTextAreaElement};

#[path = "hero_creation_choices.rs"]
mod creation_choices;
#[path = "hero_draft_storage.rs"]
mod draft_storage;
#[path = "hero_portrait_state.rs"]
mod portrait_state;
use creation_choices::{group_available, option_available};
use prost::Message;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IdentityDraft {
    pub(super) public_description: String,
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
    pub(super) portrait_scenery: String,
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
            public_description: look.public_description.clone(),
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
        let option_id_for_group = |group_id: &str| -> Result<String, JsValue> {
            let mut group_choices = self
                .appearance
                .iter()
                .filter(|choice| choice.group_id == group_id);
            let choice = group_choices
                .next()
                .ok_or_else(|| JsValue::from_str("Choose all your hero's details."))?;
            if group_choices.next().is_some() || choice.option_id.is_empty() {
                return Err(JsValue::from_str("Choose one option for each hero detail."));
            }
            Ok(choice.option_id.clone())
        };
        Ok(rpc::HeroIdentity {
            appearance: Some(rpc::HeroAppearance {
                public_description: self.public_description.clone(),
                presentation: option_id_for_group("presentation")?,
                pronouns: option_id_for_group("pronouns")?,
                age: option_id_for_group("age")?,
                build: option_id_for_group("build")?,
                skin: option_id_for_group("skin")?,
                hair: option_id_for_group("hair")?,
                hair_color: option_id_for_group("hair-color")?,
                eyes: option_id_for_group("eyes")?,
                mark: option_id_for_group("mark")?,
                attire: option_id_for_group("attire")?,
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
            portrait_scenery: self.portrait_scenery.clone(),
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
    Portrait {
        draft_version: u64,
        current: HeroDraft,
        regenerate: bool,
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
    scenery: HtmlSelectElement,
    scenery_options: Vec<rpc::JourneyOption>,
    scenery_custom: HtmlTextAreaElement,
    supported_designs: Vec<rpc::SupportedCharacterDesign>,
    design_status: Element,
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
    missing_summary: Element,
    chips: Element,
    stage_name: Element,
    step_buttons: Vec<Element>,
    progress: Element,
    stage_preview: Rc<RefCell<Element>>,
    incoming_preview: Rc<RefCell<Option<Element>>>,
    portrait_shadow: Element,
    portrait_status: Element,
    portrait_button: HtmlButtonElement,
    portrait_version: Rc<Cell<Option<u64>>>,
    portrait_pending: Rc<Cell<bool>>,
    portrait_operation: Rc<Cell<u64>>,
    look_signature: Rc<RefCell<String>>,
    look_version: Rc<Cell<u64>>,
    custom_summary: Element,
    confirm: HtmlButtonElement,
    version: Rc<Cell<u64>>,
    pending: Rc<Cell<bool>>,
    text_reviewed: Rc<Cell<bool>>,
    draft_timer: Rc<RefCell<Option<gloo_timers::callback::Timeout>>>,
    storage_status: Element,
    loaded_hero: Rc<RefCell<Option<draft_storage::LoadedHero>>>,
    selected_species: Rc<RefCell<String>>,
}

fn choices(controls: &[(rpc::JourneyGroup, HtmlSelectElement)]) -> Vec<rpc::JourneyChoice> {
    let mut selected = controls
        .iter()
        .map(|(group, select)| rpc::JourneyChoice {
            group_id: group.group_id.clone(),
            option_id: select.value(),
        })
        .collect();
    let groups = controls
        .iter()
        .map(|(group, _)| group.clone())
        .collect::<Vec<_>>();
    creation_choices::retain_available(&groups, &mut selected);
    selected
}

const NAME_HELP: &str =
    "One name or several, up to 32 letters. Spaces, apostrophes and hyphens are welcome.";
const NAME_RULE: &str = "Use letters for your hero's name, with single spaces, apostrophes or hyphens between them (up to 32)";

fn missing_choices(
    name: &str,
    groups: &[rpc::JourneyGroup],
    choices: &[rpc::JourneyChoice],
) -> Vec<String> {
    let mut missing = Vec::new();
    if name.trim().is_empty() {
        missing.push("Name your hero".to_owned());
    } else if !df_rules::local_journey::valid_character_name(name) {
        missing.push(NAME_RULE.to_owned());
    }
    missing.extend(creation_choices::missing_groups(groups, choices));
    missing
}

impl Controls {
    fn scenery_text(&self) -> String {
        let selected = self.scenery.value();
        if selected == "custom" {
            self.scenery_custom.value()
        } else {
            self.scenery_options
                .iter()
                .find(|option| option.option_id == selected)
                .map(|option| option.description.clone())
                .unwrap_or_default()
        }
    }
    fn scenery_complete(&self) -> bool {
        let value = self.scenery_text();
        self.scenery_options
            .iter()
            .any(|option| option.option_id == self.scenery.value())
            && (self.scenery.value() != "custom" || !value.trim().is_empty())
            && value.len() <= 768
            && value.chars().count() <= 360
    }
    fn design_complete(&self) -> bool {
        ["species", "class", "background"].iter().all(|id| {
            self.choices
                .iter()
                .find(|(group, _)| group.group_id == *id)
                .is_some_and(|(group, input)| {
                    let selected = input.value();
                    group
                        .options
                        .iter()
                        .any(|option| option.option_id == selected)
                })
        })
    }

    fn set_portrait_pending(&self, pending: bool) {
        self.portrait_pending.set(pending);
        self.portrait_button.set_disabled(
            pending || self.pending.get() || !self.design_complete() || !self.scenery_complete(),
        );
        let shown = portrait_state::visibility(pending, self.portrait_version.get().is_some());
        if shown.animate {
            let _ = self.portrait_shadow.set_attribute("data-pending", "true");
        } else {
            let _ = self.portrait_shadow.remove_attribute("data-pending");
        }
        if shown.shadow {
            let _ = self.portrait_shadow.remove_attribute("hidden");
        } else {
            let _ = self.portrait_shadow.set_attribute("hidden", "");
        }
        if shown.image {
            let _ = self.stage_preview.borrow().remove_attribute("hidden");
        } else {
            let _ = self.stage_preview.borrow().set_attribute("hidden", "");
        }
    }
    fn draft(&self) -> HeroDraft {
        HeroDraft {
            name: self.name.value(),
            portrait_scenery: self.scenery_text(),
            choices: choices(&self.choices),
            identity: IdentityDraft {
                public_description: self.appearance_text.value(),
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
        let species = self
            .choices
            .iter()
            .find(|(group, _)| group.group_id == "species")
            .map(|(_, input)| input.value())
            .unwrap_or_default();
        let changed_species = !self.selected_species.borrow().is_empty()
            && *self.selected_species.borrow() != species;
        *self.selected_species.borrow_mut() = species;
        let raw = self
            .choices
            .iter()
            .map(|(group, input)| rpc::JourneyChoice {
                group_id: group.group_id.clone(),
                option_id: input.value(),
            })
            .collect::<Vec<_>>();
        for (group, input) in &self.choices {
            if group
                .options
                .iter()
                .any(|option| option.requires_group == "species")
            {
                if changed_species || !group_available(group, &raw) {
                    input.set_value("");
                }
                if let [only] = group.options.as_slice()
                    && option_available(only, &raw)
                {
                    input.set_value(&only.option_id);
                }
            }
        }
        let draft = self.draft();
        if let Some(label) = self.scenery_custom.parent_element() {
            if self.scenery.value() == "custom" {
                let _ = label.remove_attribute("hidden");
            } else {
                let _ = label.set_attribute("hidden", "");
            }
        }
        let selected = |group: &str| {
            draft
                .choices
                .iter()
                .find(|choice| choice.group_id == group)
                .map(|choice| choice.option_id.as_str())
                .unwrap_or("")
        };
        let supported = self.supported_designs.iter().any(|design| {
            design.species_id == selected("species")
                && design.class_id == selected("class")
                && design.background_id == selected("background")
        });
        self.design_status.set_text_content(Some(if supported {
            "This combination is available for this adventure. Starting equipment and rules choices are shown below."
        } else {
            "You can design and generate this character. Gameplay rules for this combination are not ready."
        }));
        for (group, input) in &self.choices {
            if let Some(label) = input.parent_element() {
                if !matches!(group.group_id.as_str(), "species" | "class" | "background") {
                    if supported && group_available(group, &draft.choices) {
                        let _ = label.remove_attribute("hidden");
                    } else {
                        let _ = label.set_attribute("hidden", "");
                    }
                }
                if let Ok(Some(description)) = label.query_selector(".hero-choice-description") {
                    let value = input.value();
                    description.set_text_content(
                        group
                            .options
                            .iter()
                            .find(|option| option.option_id == value)
                            .map(|option| option.description.as_str()),
                    );
                }
            }
        }
        let groups: Vec<_> = self
            .choices
            .iter()
            .filter(|(group, _)| {
                supported || matches!(group.group_id.as_str(), "species" | "class" | "background")
            })
            .map(|(group, _)| group.clone())
            .collect();
        let mut missing = missing_choices(&draft.name, &groups, &draft.choices);
        let name_invalid =
            !draft.name.is_empty() && !df_rules::local_journey::valid_character_name(&draft.name);
        let _ = self
            .name
            .set_attribute("aria-invalid", if name_invalid { "true" } else { "false" });
        if let Some(help) = self
            .name
            .parent_element()
            .and_then(|label| label.next_element_sibling())
            .filter(|help| help.class_name() == "hero-name-help")
        {
            help.set_text_content(Some(if name_invalid { NAME_RULE } else { NAME_HELP }));
            let _ = help.set_attribute("data-invalid", if name_invalid { "true" } else { "false" });
        }
        if !self.text_reviewed.get()
            && (!self.appearance_text.value().trim().is_empty()
                || !self.backstory_text.value().trim().is_empty())
        {
            missing.push("Use your text to suggest choices and review them".to_owned());
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
        let stages = [
            missing_choices(&draft.name, &groups, &draft.identity.appearance).is_empty()
                && (self.text_reviewed.get() || self.appearance_text.value().trim().is_empty()),
            !draft.identity.origin.trim().is_empty()
                && !draft.identity.occupation.trim().is_empty()
                && !draft.identity.motivation.trim().is_empty()
                && !draft.identity.backstory.trim().is_empty()
                && draft.identity.facts.len() >= 3
                && (self.text_reviewed.get() || self.backstory_text.value().trim().is_empty()),
            supported
                && missing_choices(
                    "hero",
                    &self
                        .choices
                        .iter()
                        .map(|(group, _)| group.clone())
                        .collect::<Vec<_>>(),
                    &draft.choices,
                )
                .is_empty(),
        ];
        for (button, complete) in self.step_buttons.iter().zip(stages) {
            let _ = button.set_attribute("data-complete", if complete { "true" } else { "false" });
        }
        self.progress.set_text_content(Some(&format!(
            "{} of 3 sections ready",
            stages.iter().filter(|ready| **ready).count()
        )));
        if !supported {
            missing.push("Gameplay rules for this combination are not ready".to_owned());
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
        for (_, select) in &self.appearance {
            if let Some(label) = select.parent_element()
                && let Ok(buttons) = label.query_selector_all(".hero-swatch")
            {
                for index in 0..buttons.length() {
                    if let Some(button) = buttons
                        .item(index)
                        .and_then(|node| node.dyn_into::<Element>().ok())
                    {
                        let selected = button.get_attribute("data-option-value").as_deref()
                            == Some(select.value().as_str());
                        let _ = button
                            .set_attribute("aria-pressed", if selected { "true" } else { "false" });
                    }
                }
            }
        }

        let signature = format!(
            "{:?}|{}|{:?}|{}",
            draft.identity.appearance,
            self.appearance_text.value(),
            draft
                .choices
                .iter()
                .filter(|choice| matches!(
                    choice.group_id.as_str(),
                    "species" | "class" | "background"
                ))
                .map(|choice| (&choice.group_id, &choice.option_id))
                .collect::<Vec<_>>(),
            draft.portrait_scenery
        );
        if *self.look_signature.borrow() != signature {
            *self.look_signature.borrow_mut() = signature;
            self.look_version
                .set(self.look_version.get().saturating_add(1));
            if let Some(incoming) = self.incoming_preview.borrow_mut().take() {
                incoming.remove();
            }
            let was_pending = self.portrait_pending.get();
            self.set_portrait_pending(false);
            if self.portrait_version.get().is_some() || was_pending {
                self.portrait_status.set_text_content(Some(
                    "Your appearance has changed. Regenerate when you are ready.",
                ));
            }
        }
        self.custom_summary
            .set_text_content(Some(&self.appearance_text.value()));
        self.stage_name
            .set_text_content(Some(if draft.name.trim().is_empty() {
                "Your adventurer"
            } else {
                &draft.name
            }));
        self.missing_summary
            .set_text_content(Some(if self.pending.get() {
                "Confirming your hero…"
            } else if ready {
                "Ready for the adventure"
            } else {
                missing
                    .first()
                    .map_or("Review your choices", String::as_str)
            }));
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
        self.portrait_button.set_disabled(
            self.pending.get()
                || self.portrait_pending.get()
                || !self.design_complete()
                || !self.scenery_complete(),
        );
        if !self.design_complete() {
            self.portrait_status.set_text_content(Some(
                "Choose your ancestry, class and background before generating a portrait.",
            ));
        } else if self.portrait_status.text_content().as_deref()
            == Some("Choose your ancestry, class and background before generating a portrait.")
        {
            self.portrait_status
                .set_text_content(Some("Choose your appearance, then generate a portrait."));
        }
        if !self.scenery_complete() {
            self.portrait_status.set_text_content(Some("Choose portrait scenery or describe a setting in up to 360 characters / 768 bytes."));
        } else if self.portrait_status.text_content().as_deref()
            == Some(
                "Choose portrait scenery or describe a setting in up to 360 characters / 768 bytes.",
            )
        {
            self.portrait_status
                .set_text_content(Some("Choose your appearance, then generate a portrait."));
        }
        self.confirm
            .set_disabled(!ready || self.pending.get() || !self.scenery_complete());
        ready && !self.pending.get() && self.scenery_complete()
    }

    fn edited(&self) {
        self.version.set(self.version.get().saturating_add(1));
        self.update_missing();
        self.autosave();
    }

    fn storage_selection(&self) -> rpc::CharacterSelection {
        let field = |id: &str| {
            self.appearance
                .iter()
                .find(|(group, _)| group.group_id == id)
                .map(|(_, input)| input.value())
                .unwrap_or_default()
        };
        let draft = self.draft();
        rpc::CharacterSelection {
            name: draft.name,
            choices: draft.choices,
            portrait_scenery: draft.portrait_scenery,
            identity: Some(rpc::HeroIdentity {
                appearance: Some(rpc::HeroAppearance {
                    public_description: self.appearance_text.value(),
                    presentation: field("presentation"),
                    pronouns: field("pronouns"),
                    age: field("age"),
                    build: field("build"),
                    skin: field("skin"),
                    hair: field("hair"),
                    hair_color: field("hair-color"),
                    eyes: field("eyes"),
                    mark: field("mark"),
                    attire: field("attire"),
                }),
                origin: draft.identity.origin,
                occupation: draft.identity.occupation,
                motivation: draft.identity.motivation,
                backstory: draft.identity.backstory,
                facts: draft.identity.facts,
            }),
        }
    }

    fn stored(&self) -> Result<draft_storage::StoredDraft, JsValue> {
        Ok(draft_storage::StoredDraft {
            version: 1,
            selection: self.storage_selection().encode_to_vec(),
            appearance_text: self.appearance_text.value(),
            backstory_text: self.backstory_text.value(),
            facts: self.facts.iter().map(HtmlTextAreaElement::value).collect(),
            story_menus: self
                .story_menus
                .iter()
                .map(|(group, select)| (group.group_id.clone(), select.value()))
                .collect(),
            scenery_option: self.scenery.value(),
            scenery_custom: self.scenery_custom.value(),
            paragraph_edited: self.paragraph_edited.get(),
            text_reviewed: self.text_reviewed.get(),
            portrait_uri: if self.portrait_version.get().is_some() {
                self.stage_preview
                    .borrow()
                    .get_attribute("src")
                    .unwrap_or_default()
            } else {
                String::new()
            },
            portrait_revision: self.portrait_version.get(),
            look_revision: self.look_version.get(),
            loaded_hero: self.loaded_hero.borrow().clone(),
        })
    }

    fn save_draft(&self) {
        let result = (|| {
            let text = self
                .stored()?
                .encode()
                .ok_or_else(|| JsValue::from_str("Draft too large"))?;
            phone_storage()?.set_item(draft_storage::DRAFT_KEY, &text)
        })();
        self.storage_status
            .set_text_content(Some(if result.is_ok() {
                "Draft saved on this device"
            } else {
                "This device could not save your draft. Keep this page open."
            }));
    }

    fn autosave(&self) {
        let controls = self.clone();
        self.draft_timer
            .borrow_mut()
            .replace(gloo_timers::callback::Timeout::new(350, move || {
                controls.save_draft();
                controls.draft_timer.borrow_mut().take();
            }));
    }

    fn apply_stored(&self, stored: &draft_storage::StoredDraft) -> Result<(), JsValue> {
        let selected = stored
            .selection()
            .ok_or_else(|| JsValue::from_str("Draft invalid"))?;
        let identity = IdentityDraft::from_rpc(
            selected
                .identity
                .as_ref()
                .ok_or_else(|| JsValue::from_str("Draft identity absent"))?,
        )?;
        *self.loaded_hero.borrow_mut() = stored.loaded_hero.clone();
        self.name.set_value(&selected.name);
        self.selected_species.borrow_mut().clear();
        for (group, input) in &self.choices {
            if let Some(choice) = selected
                .choices
                .iter()
                .find(|choice| choice.group_id == group.group_id)
            {
                // Preserve unavailable carried catalog ids visibly; validation still refuses them.
                if !group
                    .options
                    .iter()
                    .any(|option| option.option_id == choice.option_id)
                    && let Some(document) = input.owner_document()
                {
                    let option = node(
                        &document,
                        input,
                        "option",
                        "",
                        &format!("{} (not available here)", choice.option_id),
                    )?;
                    option.set_attribute("value", &choice.option_id)?;
                }
                input.set_value(&choice.option_id);
            } else if group
                .options
                .iter()
                .any(|option| option.requires_group == "species")
            {
                input.set_value("");
            }
        }
        for (group, input) in &self.appearance {
            if let Some(choice) = identity
                .appearance
                .iter()
                .find(|choice| choice.group_id == group.group_id)
            {
                input.set_value(&choice.option_id);
            }
        }
        self.story.origin.set_value(&identity.origin);
        self.story.occupation.set_value(&identity.occupation);
        self.story.motivation.set_value(&identity.motivation);
        self.paragraph.set_value(&identity.backstory);
        for (input, text) in self.facts.iter().zip(&stored.facts) {
            input.set_value(text);
        }
        for (group, input) in &self.story_menus {
            if let Some((_, text)) = stored
                .story_menus
                .iter()
                .find(|(id, _)| id == &group.group_id)
            {
                input.set_value(text);
            }
        }
        self.appearance_text.set_value(&stored.appearance_text);
        self.backstory_text.set_value(&stored.backstory_text);
        self.scenery.set_value(&stored.scenery_option);
        self.scenery_custom.set_value(&stored.scenery_custom);
        self.paragraph_edited.set(stored.paragraph_edited);
        self.text_reviewed.set(stored.text_reviewed);
        self.version.set(self.version.get().saturating_add(1));
        self.update_missing();
        self.look_version.set(stored.look_revision);
        Ok(())
    }
}

fn phone_storage() -> Result<web_sys::Storage, JsValue> {
    web_sys::window()
        .ok_or_else(|| JsValue::from_str("Browser unavailable"))?
        .local_storage()?
        .ok_or_else(|| JsValue::from_str("Storage unavailable"))
}

pub(super) struct HeroCreator {
    root: Element,
    controls: Controls,
    listeners: Vec<Listener>,
    portrait_listeners: RefCell<Vec<Listener>>,
    placeholder_url: String,
    feedback: Element,
    initial_draft: draft_storage::StoredDraft,
    pagehide: Option<Closure<dyn FnMut(web_sys::Event)>>,
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

fn select_step(sections: &[(Element, Element)], active: usize) {
    for (index, (button, panel)) in sections.iter().enumerate() {
        let selected = index == active;
        let _ = button.set_attribute("aria-pressed", if selected { "true" } else { "false" });
        if selected {
            let _ = panel.remove_attribute("hidden");
        } else {
            let _ = panel.set_attribute("hidden", "");
        }
    }
}

fn set_library_visibility(picker: &Element, has_saved_heroes: bool) -> Result<(), JsValue> {
    if has_saved_heroes {
        picker.remove_attribute("hidden")
    } else {
        picker.set_attribute("hidden", "")
    }
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
    select.set_attribute("aria-label", &group.label)?;
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
        let masthead = node(document, &root, "header", "hero-masthead", "")?;
        node(document, &masthead, "span", "hero-brand", "DungeonFlux")?;
        node(
            document,
            &masthead,
            "span",
            "hero-session",
            "Adventurer’s workshop",
        )?;
        let layout = node(document, &root, "div", "hero-layout", "")?;
        let stage = node(document, &layout, "aside", "hero-stage", "")?;
        stage.set_attribute("aria-label", "Character portrait")?;
        let frame = node(document, &stage, "div", "hero-ritual-frame", "")?;
        frame.set_attribute("aria-hidden", "true")?;
        let stage_preview = node(document, &stage, "img", "hero-figure", "")?;
        stage_preview.set_attribute("alt", "Your generated character portrait")?;
        stage_preview.set_attribute("hidden", "")?;
        let portrait_shadow = node(document, &stage, "div", "hero-portrait-shadow", "")?;
        portrait_shadow.set_attribute("aria-hidden", "true")?;
        let placeholder_bytes = js_sys::Uint8Array::from(
            include_bytes!("../../../../assets/ui/generated/creator-shadow-placeholder.png")
                .as_slice(),
        );
        let parts = js_sys::Array::new();
        parts.push(&placeholder_bytes.buffer());
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("image/png");
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)?;
        let placeholder_url = web_sys::Url::create_object_url_with_blob(&blob)?;
        let placeholder = node(document, &portrait_shadow, "img", "hero-shadow-art", "")?;
        placeholder.set_attribute("src", &placeholder_url)?;
        placeholder.set_attribute("alt", "")?;
        node(document, &portrait_shadow, "div", "hero-portrait-orbit", "")?;
        let caption = node(document, &stage, "div", "hero-stage-caption", "")?;
        let stage_name = node(document, &caption, "h2", "", "Your adventurer")?;
        node(
            document,
            &caption,
            "p",
            "hero-art-label",
            "Portrait (optional)",
        )?;
        node(
            document,
            &caption,
            "p",
            "hero-art-note",
            "Your hero works without one. You can add a portrait later.",
        )?;
        let portrait_status = node(
            document,
            &caption,
            "p",
            "hero-art-status",
            "Choose your appearance, then generate a portrait.",
        )?;
        portrait_status.set_attribute("aria-live", "polite")?;
        let portrait_button = node(
            document,
            &caption,
            "button",
            "hero-portrait-button",
            "Generate portrait",
        )?
        .dyn_into::<HtmlButtonElement>()?;
        portrait_button.set_type("button");
        let editor = node(document, &layout, "div", "hero-editor", "")?;
        node(document, &editor, "h1", "", "Bring your hero to life")?;
        node(
            document,
            &editor,
            "p",
            "hero-intro",
            "Give them a face. A past. A reason to step into the unknown.",
        )?;
        let name = text_input(
            document,
            &editor,
            "Name your hero",
            "",
            df_rules::local_journey::MAXIMUM_CHARACTER_NAME_CHARACTERS as u32,
        )?;
        name.set_placeholder("Your hero’s name");
        let name_help = node(document, &editor, "p", "hero-name-help", NAME_HELP)?;
        name_help.set_attribute("aria-live", "polite")?;
        let basics = node(document, &editor, "div", "hero-basics", "")?;
        let design_status = node(document, &editor, "p", "hero-design-status", "")?;
        design_status.set_attribute("aria-live", "polite")?;
        let tablist = node(document, &editor, "div", "hero-tabs", "")?;
        tablist.set_attribute("role", "group")?;
        tablist.set_attribute("aria-label", "Character details")?;
        let mut tabs = Vec::new();
        for (step, (id, label)) in [
            ("appearance", "Look"),
            ("story", "Story"),
            ("build", "Abilities"),
        ]
        .into_iter()
        .enumerate()
        {
            let button = node(document, &tablist, "button", "hero-tab", "")?;
            node(
                document,
                &button,
                "span",
                "hero-step-number",
                &(step + 1).to_string(),
            )?;
            node(document, &button, "span", "hero-step-label", label)?;
            button.set_attribute("type", "button")?;
            button.set_attribute(
                "aria-pressed",
                if id == "appearance" { "true" } else { "false" },
            )?;
            let panel = node(document, &editor, "section", "hero-panel", "")?;
            panel.set_attribute("aria-label", label)?;
            if id != "appearance" {
                panel.set_attribute("hidden", "")?;
            }
            tabs.push((button, panel));
        }
        let progress = node(
            document,
            &editor,
            "p",
            "hero-progress",
            "0 of 3 sections ready",
        )?;
        progress.set_attribute("aria-live", "polite")?;
        let look = tabs[0].1.clone();
        let past = tabs[1].1.clone();
        let advanced = tabs[2].1.clone();
        node(
            document,
            &advanced,
            "h2",
            "hero-section-title",
            "Your starting equipment",
        )?;
        node(
            document,
            &advanced,
            "p",
            "hero-help",
            "These choices come from this adventure’s rules and starting path.",
        )?;
        let mut build = Vec::new();
        for group in offer
            .groups
            .iter()
            .filter(|group| group.group_id != "portrait")
        {
            let parent = if matches!(
                group.group_id.as_str(),
                "species" | "class" | "background" | "build-package"
            ) {
                &basics
            } else {
                &advanced
            };
            let supported = offer.supported_designs.first();
            let default = supported.and_then(|design| match group.group_id.as_str() {
                "species" => Some(design.species_id.as_str()),
                "class" => Some(design.class_id.as_str()),
                "background" => Some(design.background_id.as_str()),
                _ => None,
            });
            let input = select(document, parent, group, default)?;
            build.push((group.clone(), input));
        }
        editor.insert_before(&stage, Some(&tablist))?;
        node(
            document,
            &look,
            "h2",
            "hero-section-title",
            "A look of your own",
        )?;
        node(
            document,
            &look,
            "p",
            "hero-help",
            "Choose the details below, or add a description in your own words.",
        )?;
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
        let scenery_options = if offer.portrait_scenery_options.is_empty() {
            vec![rpc::JourneyOption {
                option_id: "studio".to_owned(),
                label: "Studio backdrop".to_owned(),
                ..Default::default()
            }]
        } else {
            offer.portrait_scenery_options.clone()
        };
        let scenery_group = rpc::JourneyGroup {
            group_id: "portrait-scenery".to_owned(),
            label: "Portrait scenery".to_owned(),
            options: scenery_options.clone(),
        };
        let scenery = select(document, &look, &scenery_group, Some("studio"))?;
        let scenery_custom =
            text_input(document, &look, "Custom portrait scenery (public)", "", 768)?;
        scenery_custom.set_rows(2);
        scenery_custom
            .set_placeholder("Mossy ruins beside a waterfall, with mist and warm morning light");
        if let Some(label) = scenery_custom.parent_element() {
            label.set_attribute("hidden", "")?;
        }
        node(
            document,
            &look,
            "p",
            "hero-help",
            "Choose the setting behind your portrait. Custom scenery is public visual direction; it does not change the adventure or your D&D background.",
        )?;
        let description = node(document, &look, "details", "hero-details", "")?;
        node(
            document,
            &description,
            "summary",
            "",
            "Describe in your own words",
        )?;
        let appearance_text = text_input(
            document,
            &description,
            "Public appearance details (optional)",
            &initial.public_description,
            4096,
        )?;
        appearance_text.set_rows(3);
        appearance_text
            .set_placeholder("A wiry grey-haired dwarf woman with a burn scar and a blue cloak");
        node(
            document,
            &description,
            "p",
            "hero-help",
            "Describe body features, facial patterns or accessibility aids. These visual details are public and saved exactly; keep private backstory elsewhere. Up to 2,000 characters / 4 KiB. Use your text to suggest menu choices, then review them before applying.",
        )?;
        let read = node(
            document,
            &description,
            "button",
            "hero-read",
            "Suggest choices from my text",
        )?;
        read.set_attribute("type", "button")?;
        let custom_summary = node(document, &look, "p", "hero-custom-summary", "")?;
        custom_summary.set_attribute("aria-label", "Exact public appearance description")?;
        let summary = node(
            document,
            &look,
            "details",
            "hero-details hero-look-summary",
            "",
        )?;
        node(
            document,
            &summary,
            "summary",
            "",
            "Your appearance at a glance",
        )?;
        let chips = node(document, &summary, "p", "hero-chips", "")?;
        node(
            document,
            &past,
            "h2",
            "hero-section-title",
            "Someone with a story",
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
        let feedback = node(document, &editor, "p", "hero-feedback", "")?;
        feedback.set_attribute("role", "status")?;
        let footer = node(document, &advanced, "div", "hero-confirm", "")?;
        node(
            document,
            &footer,
            "p",
            "hero-help",
            "Your look and story will be shared at the table.",
        )?;
        let validation = node(document, &footer, "details", "hero-validation", "")?;
        let missing_summary = node(document, &validation, "summary", "", "Review your choices")?;
        missing_summary.set_attribute("aria-live", "polite")?;
        let missing = node(document, &validation, "p", "hero-missing", "")?;
        missing.set_attribute("aria-live", "polite")?;
        let confirm = node(document, &footer, "button", "", "Join the table")?
            .dyn_into::<HtmlButtonElement>()?;
        confirm.set_type("button");
        let controls = Controls {
            name,
            scenery,
            scenery_custom,
            scenery_options,
            supported_designs: offer.supported_designs.clone(),
            design_status,
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
            missing_summary,
            chips,
            stage_name,
            step_buttons: tabs.iter().map(|(button, _)| button.clone()).collect(),
            progress,
            stage_preview: Rc::new(RefCell::new(stage_preview)),
            incoming_preview: Rc::new(RefCell::new(None)),
            portrait_shadow,
            portrait_status,
            portrait_button,
            portrait_version: Rc::new(Cell::new(None)),
            portrait_pending: Rc::new(Cell::new(false)),
            portrait_operation: Rc::new(Cell::new(0)),
            look_signature: Rc::new(RefCell::new(String::new())),
            look_version: Rc::new(Cell::new(0)),
            custom_summary,
            confirm,
            version: Rc::new(Cell::new(0)),
            pending: Rc::new(Cell::new(false)),
            text_reviewed: Rc::new(Cell::new(true)),
            draft_timer: Rc::new(RefCell::new(None)),
            loaded_hero: Rc::new(RefCell::new(None)),
            selected_species: Rc::new(RefCell::new(String::new())),
            storage_status: node(
                document,
                &footer,
                "p",
                "hero-draft-status",
                "Drafts and saved heroes are kept on this device.",
            )?,
        };
        let initial_draft = controls.stored()?;
        let on_event = Rc::new(on_event);
        let mut surface = Self {
            root,
            controls,
            listeners: Vec::new(),
            portrait_listeners: RefCell::new(Vec::new()),
            placeholder_url,
            feedback,
            initial_draft,
            pagehide: None,
        };
        for (active, (button, _)) in tabs.iter().enumerate() {
            let sections = tabs.clone();
            surface.listen(button.clone(), "click", move |_| {
                select_step(&sections, active);
            })?;
        }
        for (index, (_, panel)) in tabs.iter().enumerate() {
            let navigation = node(document, panel, "div", "hero-step-navigation", "")?;
            if index > 0 {
                let back = node(document, &navigation, "button", "hero-step-back", "Back")?;
                back.set_attribute("type", "button")?;
                let sections = tabs.clone();
                surface.listen(back, "click", move |_| {
                    select_step(&sections, index - 1);
                })?;
            }
            if index < 2 {
                let label = ["Next: Story", "Next: Abilities"][index];
                let next = node(document, &navigation, "button", "hero-step-next", label)?;
                next.set_attribute("type", "button")?;
                let sections = tabs.clone();
                surface.listen(next, "click", move |_| {
                    select_step(&sections, index + 1);
                })?;
            } else {
                navigation.append_child(&surface.controls.confirm)?;
            }
        }
        // Swatches supplement the actual selects, so keyboard/native-picker behavior remains.
        for (group, select) in surface.controls.appearance.clone() {
            if !matches!(group.group_id.as_str(), "skin" | "hair-color") {
                continue;
            }
            let Some(label) = select.parent_element() else {
                continue;
            };
            let swatches = node(document, &label, "div", "hero-swatches", "")?;
            swatches.set_attribute("role", "group")?;
            swatches.set_attribute("aria-label", &format!("{} quick choices", group.label))?;
            for option in &group.options {
                let color = match (group.group_id.as_str(), option.option_id.as_str()) {
                    (_, "pale") => "#e6c5a6",
                    (_, "tan") => "#ba8762",
                    (_, "olive") => "#9d8058",
                    ("skin", "brown") => "#82553d",
                    (_, "dark" | "deep-brown") => "#493129",
                    (_, "grey") => "#9ca4aa",
                    (_, "green") => "#79865b",
                    (_, "blue") => "#607e98",
                    (_, "black") => "#25252a",
                    (_, "brown") => "#694932",
                    (_, "red") => "#a05a3e",
                    (_, "blond") => "#d4b97c",
                    (_, "auburn") => "#793d2c",
                    (_, "white") => "#e9e5dc",
                    _ => "#a79a84",
                };
                let button = node(document, &swatches, "button", "hero-swatch", "")?;
                button.set_attribute("type", "button")?;
                button
                    .set_attribute("aria-label", &format!("{}: {}", group.label, option.label))?;
                button.set_attribute("title", &option.label)?;
                button.set_attribute("data-option-value", &option.option_id)?;
                button.set_attribute("style", &format!("--swatch:{color}"))?;
                button.set_attribute(
                    "aria-pressed",
                    if select.value() == option.option_id {
                        "true"
                    } else {
                        "false"
                    },
                )?;
                let control = select.clone();
                let choice = option.option_id.clone();
                let controls = surface.controls.clone();
                let siblings = swatches.clone();
                surface.listen(button.clone(), "click", move |_| {
                    if controls.pending.get() {
                        return;
                    }
                    control.set_value(&choice);
                    if let Ok(buttons) = siblings.query_selector_all("button") {
                        for index in 0..buttons.length() {
                            if let Some(item) = buttons
                                .item(index)
                                .and_then(|node| node.dyn_into::<Element>().ok())
                            {
                                let _ = item.set_attribute("aria-pressed", "false");
                            }
                        }
                    }
                    let _ = button.set_attribute("aria-pressed", "true");
                    controls.edited();
                })?;
            }
        }
        // Native controls keep phone pickers and keyboard navigation. Descriptions follow
        // the current server catalog selection rather than a wall of ancestry cards.
        for (group, input) in &surface.controls.choices {
            if matches!(group.group_id.as_str(), "species" | "class" | "background")
                && let Some(label) = input.parent_element()
            {
                node(document, &label, "span", "hero-choice-description", "")?;
            }
        }
        let inputs = surface.root.query_selector_all("textarea, select")?;
        for index in 0..inputs.length() {
            let Some(input) = inputs.item(index) else {
                continue;
            };
            let element = input.dyn_into::<Element>()?;
            let controls = surface.controls.clone();
            surface.listen(element.clone(), "input", move |_| controls.edited())?;
            let controls = surface.controls.clone();
            surface.listen(element, "change", move |_| controls.edited())?;
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
        surface.listen(
            controls.portrait_button.clone().unchecked_into(),
            "click",
            move |_| {
                if controls.pending.get()
                    || controls.portrait_pending.get()
                    || !controls.design_complete()
                    || !controls.scenery_complete()
                {
                    controls.update_missing();
                    return;
                }
                if let Some(image) = controls.incoming_preview.borrow_mut().take() {
                    image.remove();
                }
                controls.set_portrait_pending(true);
                controls
                    .portrait_status
                    .set_text_content(Some("Creating your portrait… You can keep editing."));
                event(CreationEvent::Portrait {
                    draft_version: controls.look_version.get(),
                    current: controls.draft(),
                    regenerate: controls.portrait_operation.get() != 0
                        || controls.portrait_version.get().is_some(),
                });
            },
        )?;
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
                if !controls.pending.get() && controls.update_missing() {
                    controls.save_draft();
                    controls.pending.set(true);
                    controls.update_missing();
                    on_event(CreationEvent::Confirm(controls.draft()));
                }
            },
        )?;
        surface.controls.update_missing();
        let restored = (|| -> Result<(), JsValue> {
            let storage = phone_storage()?;
            if let Some(text) = storage.get_item(draft_storage::DRAFT_KEY)? {
                if let Some(stored) = draft_storage::StoredDraft::decode(&text) {
                    if surface.controls.apply_stored(&stored).is_ok() {
                        surface
                            .controls
                            .storage_status
                            .set_text_content(Some("Draft restored from this device"));
                        if !stored.portrait_uri.is_empty() {
                            surface.load_portrait_reference(
                                stored.look_revision,
                                &stored.portrait_uri,
                                stored.portrait_revision.unwrap_or(stored.look_revision),
                            )?;
                        }
                    } else {
                        storage.remove_item(draft_storage::DRAFT_KEY)?;
                    }
                } else {
                    storage.remove_item(draft_storage::DRAFT_KEY)?;
                }
            }
            Ok(())
        })();
        if restored.is_err() {
            surface.controls.storage_status.set_text_content(Some(
                "This device could not restore your draft. You can still make a hero.",
            ));
        }
        if let Some(window) = web_sys::window() {
            let controls = surface.controls.clone();
            let callback = Closure::wrap(Box::new(move |_: web_sys::Event| {
                controls.draft_timer.borrow_mut().take();
                controls.save_draft();
            }) as Box<dyn FnMut(web_sys::Event)>);
            window
                .add_event_listener_with_callback("pagehide", callback.as_ref().unchecked_ref())?;
            surface.pagehide = Some(callback);
        }
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

    pub(super) fn current_draft(&self) -> HeroDraft {
        self.controls.draft()
    }

    pub(super) fn dispose(self) -> Result<(), JsValue> {
        if self.controls.draft_timer.borrow_mut().take().is_some() {
            self.controls.save_draft();
        }
        self.root.remove();
        // Dropping the surface releases the listeners it owns, independently of page redraws.
        Ok(())
    }

    /// Confirmation stays pending across same-phase view redraws and portrait polling.
    pub(super) fn is_pending(&self) -> bool {
        self.controls.pending.get()
    }

    /// The caller clears pending only after a durable outcome or a preparation failure.
    pub(super) fn set_pending(&self, pending: bool) {
        self.controls.pending.set(pending);
        self.controls.update_missing();
        if let Ok(inputs) = self.root.query_selector_all("textarea, select, button") {
            for index in 0..inputs.length() {
                if let Some(input) = inputs
                    .item(index)
                    .and_then(|node| node.dyn_into::<Element>().ok())
                {
                    if pending && !input.has_attribute("disabled") {
                        let _ = input.set_attribute("data-lock-disabled", "");
                        let _ = input.set_attribute("disabled", "");
                    } else if !pending && input.has_attribute("data-lock-disabled") {
                        let _ = input.remove_attribute("disabled");
                        let _ = input.remove_attribute("data-lock-disabled");
                    }
                }
            }
        }
    }

    pub(super) fn look_version(&self) -> u64 {
        self.controls.look_version.get()
    }

    pub(super) fn has_current_portrait(&self) -> bool {
        self.controls.portrait_version.get() == Some(self.controls.look_version.get())
    }

    pub(super) fn keep_draft(&self) {
        self.controls.draft_timer.borrow_mut().take();
        self.controls.save_draft();
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

    pub(super) fn observe_portrait_operation(&self, operation_hash: u64) {
        self.controls.portrait_operation.set(operation_hash);
        self.controls
            .portrait_button
            .set_text_content(Some("Regenerate portrait"));
    }

    pub(super) fn observe_portrait_visibility(&self, hidden: bool) {
        if self.controls.portrait_pending.get() {
            super::presentation_timing::diagnostic(
                &self.root,
                if hidden {
                    crate::client_diagnostics::Diagnostic::PortraitPageHidden
                } else {
                    crate::client_diagnostics::Diagnostic::PortraitPageVisible
                },
                self.controls.portrait_operation.get(),
                0,
                0,
            );
        }
    }

    pub(super) fn apply_portrait(&self, version: u64, state: &str, uri: &str, message: &str) {
        let controls = &self.controls;
        if controls.look_version.get() != version {
            super::presentation_timing::diagnostic(
                &self.root,
                crate::client_diagnostics::Diagnostic::PortraitIgnoredRevision,
                controls.portrait_operation.get(),
                0,
                0,
            );
            return;
        }
        if state == "ready" && uri.starts_with("/gameplay/creator-portrait/") && uri.len() <= 128 {
            super::presentation_timing::diagnostic(
                &self.root,
                crate::client_diagnostics::Diagnostic::PortraitApplied,
                controls.portrait_operation.get(),
                0,
                0,
            );
            if self.load_portrait(version, uri).is_err() {
                super::presentation_timing::diagnostic(
                    &self.root,
                    crate::client_diagnostics::Diagnostic::PortraitImageFailed,
                    controls.portrait_operation.get(),
                    0,
                    0,
                );
                controls.set_portrait_pending(false);
                controls.portrait_status.set_text_content(Some("The portrait image could not load. Your previous portrait is kept. Choose Regenerate to try again."));
            }
            return;
        }
        controls.set_portrait_pending(state == "pending");
        controls.portrait_status.set_text_content(Some(if state == "failed" && message.trim().is_empty() {
            "Generation failed. Your draft and previous portrait are kept. Edit your appearance or choose Regenerate to send a new request."
        } else {message}));
    }

    fn load_portrait(&self, version: u64, uri: &str) -> Result<(), JsValue> {
        self.load_portrait_reference(version, uri, version)
    }

    fn load_portrait_reference(
        &self,
        version: u64,
        uri: &str,
        portrait_revision: u64,
    ) -> Result<(), JsValue> {
        self.portrait_listeners.borrow_mut().clear();
        if let Some(previous) = self.controls.incoming_preview.borrow_mut().take() {
            previous.remove();
        }
        let document = self
            .root
            .owner_document()
            .ok_or_else(|| JsValue::from_str("creation document unavailable"))?;
        let parent = self
            .controls
            .stage_preview
            .borrow()
            .parent_element()
            .ok_or_else(|| JsValue::from_str("portrait stage unavailable"))?;
        let image = node(&document, &parent, "img", "hero-figure", "")?;
        parent.insert_before(&image, Some(&self.controls.portrait_shadow))?;
        image.set_attribute("alt", "Your generated character portrait")?;
        image.set_attribute("hidden", "")?;
        self.controls.incoming_preview.replace(Some(image.clone()));
        self.controls.set_portrait_pending(true);
        self.controls
            .portrait_status
            .set_text_content(Some("Your portrait is ready. Loading image…"));
        for event in ["load", "error"] {
            let root = self.root.clone();
            let operation_hash = self.controls.portrait_operation.get();
            let controls = self.controls.clone();
            let current = image.clone();
            let callback = Closure::wrap(Box::new(move |_: web_sys::Event| {
                let matches = controls
                    .incoming_preview
                    .borrow()
                    .as_ref()
                    .is_some_and(|incoming| incoming.is_same_node(Some(&current)));
                if !matches || controls.look_version.get() != version {
                    super::presentation_timing::diagnostic(
                        &root,
                        crate::client_diagnostics::Diagnostic::PortraitIgnoredRevision,
                        operation_hash,
                        0,
                        0,
                    );
                    return;
                }
                controls.incoming_preview.borrow_mut().take();
                if event == "load" {
                    controls.stage_preview.borrow().remove();
                    controls.stage_preview.replace(current.clone());
                    controls.portrait_version.set(Some(portrait_revision));
                    controls.autosave();
                    let _ = current.set_attribute("data-loaded", "true");
                    controls.set_portrait_pending(false);
                    super::presentation_timing::diagnostic(
                        &root,
                        crate::client_diagnostics::Diagnostic::PortraitImageLoaded,
                        operation_hash,
                        0,
                        0,
                    );
                    super::presentation_timing::portrait_display(&root, &current, operation_hash);
                    controls
                        .portrait_button
                        .set_text_content(Some("Regenerate portrait"));
                    controls.portrait_status.set_text_content(Some(
                        if portrait_revision == version {
                            "Your portrait is ready."
                        } else {
                            "Your look has changed. Regenerate to update this portrait."
                        },
                    ));
                } else {
                    super::presentation_timing::diagnostic(
                        &root,
                        crate::client_diagnostics::Diagnostic::PortraitImageFailed,
                        operation_hash,
                        0,
                        0,
                    );
                    current.remove();
                    controls.set_portrait_pending(false);
                    controls.portrait_status.set_text_content(Some("The portrait image could not load. Your previous portrait is kept. Choose Regenerate to try again."));
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            image.add_event_listener_with_callback(event, callback.as_ref().unchecked_ref())?;
            self.portrait_listeners.borrow_mut().push(Listener {
                element: image.clone(),
                event,
                callback,
            });
        }
        image.set_attribute("src", uri)?;
        Ok(())
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
                public_description: String::new(),
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
            portrait_scenery: String::new(),
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
            "Maren  Coast",
            "123",
            " Maren",
            "abcdefghijklmnopqrstuvwxyzabcdefg",
        ] {
            assert_eq!(missing_choices(name, &[], &[]), [NAME_RULE]);
        }
        for name in [
            "Maren-Coast",
            "Maren of the Coast",
            "Mären",
            "Vix Ashlantern",
        ] {
            assert!(missing_choices(name, &[], &[]).is_empty(), "{name:?}");
        }
    }

    #[test]
    fn saved_heroes_are_labelled_by_species_and_class() {
        assert_eq!(library_design_label("dwarf", "fighter-1"), "Dwarf fighter");
        assert_eq!(
            library_design_label("half-elf", "eldritch-knight-1"),
            "Half-elf eldritch knight"
        );
        assert_eq!(library_design_label("", ""), "");
    }

    #[test]
    fn empty_saved_hero_shelves_stay_hidden_until_they_have_a_hero() {
        let document = web_sys::window().unwrap().document().unwrap();
        let picker = document.create_element("section").unwrap();

        set_library_visibility(&picker, false).unwrap();
        assert!(picker.has_attribute("hidden"));
        set_library_visibility(&picker, true).unwrap();
        assert!(!picker.has_attribute("hidden"));
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
            public_description: String::new(),
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
                CreationEvent::Portrait {
                    draft_version,
                    current,
                    ..
                } => {
                    let _ = (draft_version, current);
                }
                CreationEvent::Confirm(draft) => assert!(!draft.name.is_empty()),
            },
        )
        .unwrap();
        assert!(
            surface
                .root()
                .query_selector(".hero-intro + .hero-field + .hero-name-help + .hero-basics")
                .unwrap()
                .is_some()
        );
        assert!(
            surface
                .root()
                .query_selector(".hero-basics ~ .hero-stage + .hero-tabs")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            surface
                .root()
                .query_selector(".hero-art-label")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Portrait (optional)")
        );
        assert!(
            surface
                .root()
                .query_selector(".hero-art-note")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap()
                .contains("works without one")
        );
        let next = surface
            .root()
            .query_selector(".hero-step-next")
            .unwrap()
            .unwrap();
        assert_eq!(next.text_content().as_deref(), Some("Next: Story"));
        next.dispatch_event(&web_sys::Event::new("click").unwrap())
            .unwrap();
        let active = surface
            .root()
            .query_selector(".hero-panel:not([hidden])")
            .unwrap()
            .unwrap();
        assert_eq!(active.get_attribute("aria-label").as_deref(), Some("Story"));
        let next = surface
            .root()
            .query_selector(
                ".hero-panel[aria-label='Story'] > .hero-step-navigation > .hero-step-next",
            )
            .unwrap()
            .unwrap();
        assert_eq!(next.text_content().as_deref(), Some("Next: Abilities"));
        next.dispatch_event(&web_sys::Event::new("click").unwrap())
            .unwrap();
        let active = surface
            .root()
            .query_selector(".hero-panel:not([hidden])")
            .unwrap()
            .unwrap();
        assert_eq!(
            active.get_attribute("aria-label").as_deref(),
            Some("Abilities")
        );
        let read = surface
            .root()
            .query_selector(".hero-read")
            .unwrap()
            .unwrap();
        assert_eq!(
            read.text_content().as_deref(),
            Some("Suggest choices from my text")
        );
        assert!(
            surface
                .root()
                .query_selector("details .hero-read")
                .unwrap()
                .is_some()
        );
        let join = surface
            .root()
            .query_selector(
                ".hero-panel[aria-label='Abilities'] > .hero-step-navigation > button:not(.hero-step-back)",
            )
            .unwrap()
            .unwrap();
        assert_eq!(join.text_content().as_deref(), Some("Join the table"));
        assert!(include_str!("hero_creator.css").contains("@media(max-width:600px)"));
        assert_eq!(
            surface
                .root()
                .query_selector_all("details")
                .unwrap()
                .length(),
            3
        );
        assert!(surface.controls.confirm.disabled());
        surface.controls.set_portrait_pending(true);
        assert_eq!(
            surface
                .controls
                .portrait_shadow
                .get_attribute("data-pending")
                .as_deref(),
            Some("true")
        );
        assert!(!surface.controls.portrait_shadow.has_attribute("hidden"));
        assert!(surface.controls.portrait_button.disabled());
        surface.controls.set_portrait_pending(false);
        assert!(
            !surface
                .controls
                .portrait_shadow
                .has_attribute("data-pending")
        );
        assert!(!surface.controls.portrait_button.disabled());
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
        assert!(surface.is_pending());
        assert!(surface.controls.name.disabled());
        assert!(surface.controls.portrait_button.disabled());
        assert!(surface.controls.confirm.disabled());
        assert!(!surface.apply_preview(2, &initial, &[]));
        surface.set_pending(false);
        assert!(!surface.is_pending());
        assert!(!surface.controls.name.disabled());
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
                .contains("Use your text to suggest choices")
        );
        let version = surface.controls.version.get();
        assert!(surface.apply_preview(version, &initial, &[]));
        assert!(!surface.controls.confirm.disabled());
        let root = surface.root().clone();
        surface.dispose().unwrap();
        assert!(root.parent_element().is_none());
    }
}

impl Drop for HeroCreator {
    fn drop(&mut self) {
        self.controls.draft_timer.borrow_mut().take();
        if let (Some(window), Some(callback)) = (web_sys::window(), self.pagehide.take()) {
            let _removed = window
                .remove_event_listener_with_callback("pagehide", callback.as_ref().unchecked_ref());
        }
        let _ = web_sys::Url::revoke_object_url(&self.placeholder_url);
    }
}

fn library_key(create: bool) -> Result<Option<Vec<u8>>, JsValue> {
    let storage = phone_storage()?;
    if let Some(text) = storage.get_item(draft_storage::LIBRARY_KEY)? {
        if let Some(key) = draft_storage::decode_library_key(&text) {
            return Ok(Some(key));
        }
        storage.remove_item(draft_storage::LIBRARY_KEY)?;
    }
    if !create {
        return Ok(None);
    }
    let mut key = [0_u8; 32];
    web_sys::window()
        .ok_or_else(|| JsValue::from_str("Browser unavailable"))?
        .crypto()?
        .get_random_values_with_u8_array(&mut key)?;
    let encoded = key
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    storage.set_item(draft_storage::LIBRARY_KEY, &encoded)?;
    Ok(Some(key.to_vec()))
}

/// Marks the saved hero now in the creator; `None` when the player starts a new hero.
fn mark_library_choice(list: &Element, chosen: Option<&Element>) {
    let Ok(items) = list.query_selector_all(".hero-library-item") else {
        return;
    };
    for index in 0..items.length() {
        if let Some(item) = items
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        {
            let pressed = chosen.is_some_and(|chosen| chosen == &item);
            let _ = item.set_attribute("aria-pressed", if pressed { "true" } else { "false" });
        }
    }
    if let Some(picker) = list.parent_element()
        && let Ok(Some(row)) = picker.query_selector(".hero-library-removal")
    {
        let _ = row.remove_attribute("data-confirming");
        let _ = row.toggle_attribute_with_force("hidden", chosen.is_none());
        if let Some(chosen) = chosen {
            let name = chosen.get_attribute("data-hero-name").unwrap_or_default();
            if let Ok(Some(quiet)) = row.query_selector(".hero-library-remove-quiet") {
                quiet.set_text_content(Some(&format!("Remove {name} from your heroes")));
            }
            if let Ok(Some(prompt)) = row.query_selector(".hero-library-remove-prompt") {
                prompt.set_text_content(Some(&format!(
                    "Remove {name} for good? Games you already played keep their copy."
                )));
            }
        }
    }
}

fn mount_library_removal(
    creator: &mut HeroCreator,
    client: &Rc<RefCell<super::Client>>,
    document: &Document,
    picker: &Element,
    list: &Element,
) -> Result<(), JsValue> {
    let row = node(document, picker, "div", "hero-library-removal", "")?;
    row.set_attribute("hidden", "")?;
    let quiet = node(document, &row, "button", "hero-library-remove-quiet", "")?;
    quiet.set_attribute("type", "button")?;
    let confirmation = node(
        document,
        &row,
        "div",
        "hero-library-remove-confirmation",
        "",
    )?;
    node(
        document,
        &confirmation,
        "p",
        "hero-library-remove-prompt",
        "",
    )?;
    let remove = node(
        document,
        &confirmation,
        "button",
        "hero-library-remove-confirm",
        "Remove",
    )?;
    remove.set_attribute("type", "button")?;
    let keep = node(
        document,
        &confirmation,
        "button",
        "hero-library-remove-keep",
        "Keep",
    )?;
    keep.set_attribute("type", "button")?;
    let status = node(document, picker, "p", "hero-library-status", "")?;
    status.set_attribute("role", "status")?;
    let confirming = row.clone();
    let focus = keep.clone();
    creator.listen(quiet.clone(), "click", move |_| {
        let _ = confirming.set_attribute("data-confirming", "true");
        if let Some(button) = focus.dyn_ref::<web_sys::HtmlElement>() {
            let _ = button.focus();
        }
    })?;
    let cancel = row.clone();
    let focus = quiet.clone();
    creator.listen(keep.clone(), "click", move |_| {
        let _ = cancel.remove_attribute("data-confirming");
        if let Some(button) = focus.dyn_ref::<web_sys::HtmlElement>() {
            let _ = button.focus();
        }
    })?;
    let cancel = row.clone();
    let focus = quiet;
    let busy = remove.clone();
    creator.listen(row.clone(), "keydown", move |event| {
        if event
            .dyn_ref::<web_sys::KeyboardEvent>()
            .is_some_and(|event| event.key() == "Escape")
            && !busy.has_attribute("disabled")
        {
            event.prevent_default();
            let _ = cancel.remove_attribute("data-confirming");
            if let Some(button) = focus.dyn_ref::<web_sys::HtmlElement>() {
                let _ = button.focus();
            }
        }
    })?;
    let weak = Rc::downgrade(client);
    let list = list.clone();
    let remove_button = remove.clone();
    creator.listen(remove, "click", move |_| {
        if remove_button.has_attribute("disabled") {
            return;
        }
        let Ok(Some(mark)) = list.query_selector(".hero-library-item[aria-pressed=true]") else {
            return;
        };
        let Some(hero_id) = mark
            .get_attribute("data-hero-id")
            .and_then(|id| id.parse::<i64>().ok())
        else {
            return;
        };
        let name = mark.get_attribute("data-hero-name").unwrap_or_default();
        let Some(owner) = weak.upgrade() else {
            return;
        };
        let state = owner.borrow();
        let Some(creator) = state.creation.as_ref() else {
            return;
        };
        if creator.controls.pending.get() {
            return;
        }
        let key = match library_key(false) {
            Ok(Some(key)) => key,
            _ => {
                status.set_text_content(Some(&format!(
                    "{name} could not be removed. Your heroes are unchanged."
                )));
                let _ = row.remove_attribute("data-confirming");
                return;
            }
        };
        creator.set_pending(true);
        let credential = state.credential.clone();
        drop(state);
        let _ = remove_button.set_attribute("disabled", "");
        let _ = keep.set_attribute("disabled", "");
        let weak = weak.clone();
        let row = row.clone();
        let list = list.clone();
        let status = status.clone();
        let remove = remove_button.clone();
        let keep = keep.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let (connection, mut library) = connect_library().await?;
                let request =
                    library_request(rpc::DeleteHeroRequest { key, hero_id }, &credential)?;
                let result = library.delete_hero(request).await;
                connection.close();
                result.map_err(|status| {
                    library_failed(&weak, 3, status.code());
                    "Hero could not be removed".to_owned()
                })
            }
            .await;
            if !row.is_connected() {
                return;
            }
            let Some(owner) = weak.upgrade() else {
                return;
            };
            {
                let state = owner.borrow();
                let Some(creator) = state.creation.as_ref() else {
                    return;
                };
                creator.set_pending(false);
                let _ = remove.remove_attribute("disabled");
                let _ = keep.remove_attribute("disabled");
                let _ = row.remove_attribute("data-confirming");
                if result.is_err() {
                    status.set_text_content(Some(&format!(
                        "{name} could not be removed. Your heroes are unchanged."
                    )));
                    return;
                }
                // A repeated delete (removed=false) is the same desired absence.
                if creator
                    .controls
                    .loaded_hero
                    .borrow()
                    .as_ref()
                    .is_some_and(|hero| hero.hero_id == hero_id)
                {
                    creator.controls.loaded_hero.borrow_mut().take();
                    creator.controls.draft_timer.borrow_mut().take();
                    creator.controls.save_draft();
                }
                mark_library_choice(&list, None);
                status.set_text_content(Some(&format!("{name} was removed from your heroes.")));
            }
            let _ = mark.class_list().add_1("hero-library-removing");
            let reduced = web_sys::window()
                .and_then(|window| window.get_computed_style(&mark).ok().flatten())
                .and_then(|style| style.get_property_value("transition-duration").ok())
                .is_some_and(|duration| duration == "0s");
            if !reduced {
                gloo_timers::future::TimeoutFuture::new(180).await;
            }
            if !row.is_connected() {
                return;
            }
            mark.remove();
            if list
                .query_selector(".hero-library-item")
                .ok()
                .flatten()
                .is_none()
                && let Some(picker) = list.parent_element()
                && let Ok(Some(note)) = picker.query_selector(".hero-library-note")
            {
                note.set_text_content(Some(
                    "Heroes you confirm are kept here for your next adventure.",
                ));
            }
        });
    })?;
    Ok(())
}

/// "Dwarf fighter" from the library row's species and class option ids.
fn library_design_label(species: &str, class: &str) -> String {
    let mut letters = species.chars();
    let species = letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect::<String>())
        .unwrap_or_default();
    let class = class.trim_end_matches("-1").replace('-', " ");
    format!("{species} {class}").trim().to_owned()
}

type LibraryClient =
    rpc::hero_library_service_client::HeroLibraryServiceClient<df_rpc_bridge::BrowserChannel>;
fn library_failed(
    weak: &std::rc::Weak<std::cell::RefCell<super::Client>>,
    operation: u32,
    code: tonic::Code,
) {
    if let Some(client) = weak.upgrade() {
        super::presentation_timing::diagnostic(
            &client.borrow().root,
            crate::client_diagnostics::Diagnostic::LibraryFailed,
            0,
            code as u32,
            operation,
        );
    }
}

async fn connect_library() -> Result<(df_rpc_bridge::BrowserConnection, LibraryClient), String> {
    let url = super::tunnel_url()?;
    let (connection, channel) = df_rpc_bridge::BrowserConnection::connect(&url)
        .await
        .map_err(|_| "Your heroes could not connect".to_owned())?;
    Ok((
        connection,
        LibraryClient::new(channel).max_decoding_message_size(df_rpc_bridge::RPC_MESSAGE_BYTES),
    ))
}
fn library_request<T>(body: T, credential: &str) -> Result<tonic::Request<T>, String> {
    let mut request = super::request(body, credential)?;
    request.set_timeout(std::time::Duration::from_secs(20));
    Ok(request)
}
fn portrait_data(summary: &rpc::LibraryHeroSummary) -> Result<String, JsValue> {
    if summary.portrait.is_empty() {
        return Ok(String::new());
    }
    if !matches!(
        summary.portrait_media_type.as_str(),
        "image/jpeg" | "image/png" | "image/webp"
    ) || summary.portrait.len() > 6 * 1024 * 1024
    {
        return Err(JsValue::from_str("Portrait invalid"));
    }
    let binary = summary
        .portrait
        .iter()
        .map(|byte| char::from(*byte))
        .collect::<String>();
    let encoded = web_sys::window()
        .ok_or_else(|| JsValue::from_str("Browser unavailable"))?
        .btoa(&binary)?;
    Ok(format!(
        "data:{};base64,{encoded}",
        summary.portrait_media_type
    ))
}
impl HeroCreator {
    fn apply_library(
        &self,
        hero: rpc::LibraryHeroIdentity,
        summary: rpc::LibraryHeroSummary,
    ) -> Result<(), JsValue> {
        let identity = hero
            .identity
            .ok_or_else(|| JsValue::from_str("Hero details unavailable"))?;
        let selection = rpc::CharacterSelection {
            name: hero.name,
            choices: hero.design,
            identity: Some(identity.clone()),
            portrait_scenery: hero.portrait_scenery.clone(),
        };
        let mut stored = self.controls.stored()?;
        stored.selection = selection.encode_to_vec();
        stored.loaded_hero = Some(draft_storage::LoadedHero {
            hero_id: summary.hero_id,
            revision: summary.revision,
        });
        stored.appearance_text = identity
            .appearance
            .as_ref()
            .map(|look| look.public_description.clone())
            .unwrap_or_default();
        stored.backstory_text.clear();
        stored.facts = identity.facts;
        #[path = "../gameplay/living_scenes/plates/scenery_selection.rs"]
        mod scenery_selection;
        match scenery_selection::saved_option(
            &self.controls.scenery_options,
            &hero.portrait_scenery,
        ) {
            Some(option) => {
                stored.scenery_option = option.to_owned();
                stored.scenery_custom.clear();
            }
            None => {
                stored.scenery_option = "custom".to_owned();
                stored.scenery_custom = hero.portrait_scenery;
            }
        }
        stored.paragraph_edited = true;
        stored.text_reviewed = true;
        stored.look_revision = self.controls.look_version.get().saturating_add(1);
        stored.portrait_revision = Some(stored.look_revision);
        self.controls.apply_stored(&stored)?;
        if summary.portrait.is_empty() {
            self.controls.portrait_version.set(None);
            self.controls.set_portrait_pending(false);
        }
        if !summary.portrait.is_empty() {
            // The server imports checked library bytes into the accepted portrait path on Load.
            let key = summary.portrait_key;
            if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(JsValue::from_str("Portrait reference invalid"));
            }
            self.load_portrait(
                stored.look_revision,
                &format!("/gameplay/library-portrait/{key}"),
            )?;
        }
        self.controls.save_draft();
        Ok(())
    }
}

thread_local! {
    static LIBRARY_SAVING: Cell<bool> = const {Cell::new(false)};
    static LIBRARY_SAVED: RefCell<String> = const {RefCell::new(String::new())};
}

/// Called by the player renderer after it has applied a current, authenticated journey.
/// The game remains playable when any library operation fails. A reconnect redraw retries.
pub(super) fn sync_library(
    client: &Rc<RefCell<super::Client>>,
    journey: &rpc::JourneyView,
) -> Result<(), JsValue> {
    if client.borrow().role != "player" {
        return Ok(());
    }
    if journey.creation.is_none()
        && let Some(own) = journey.own_character.as_ref()
    {
        let credential = client.borrow().credential.clone();
        let stored_origin = (|| -> Result<_, JsValue> {
            let storage = phone_storage()?;
            let draft = if let Some(creator) = client.borrow().creation.as_ref() {
                creator.controls.draft_timer.borrow_mut().take();
                Some(creator.controls.stored()?)
            } else {
                storage
                    .get_item(draft_storage::DRAFT_KEY)?
                    .as_deref()
                    .and_then(draft_storage::StoredDraft::decode)
            };
            let origin = if let Some(draft) = draft {
                let origin = draft_storage::SeatOrigin {
                    credential: credential.clone(),
                    loaded_hero: draft.loaded_hero,
                };
                storage.set_item(
                    draft_storage::ORIGIN_KEY,
                    &origin
                        .encode()
                        .ok_or_else(|| JsValue::from_str("Hero reference invalid"))?,
                )?;
                Some(origin)
            } else {
                storage
                    .get_item(draft_storage::ORIGIN_KEY)?
                    .as_deref()
                    .and_then(|text| draft_storage::SeatOrigin::decode(text, &credential))
            };
            storage.remove_item(draft_storage::DRAFT_KEY)?;
            Ok(origin
                .and_then(|origin| origin.loaded_hero)
                .map(|origin| origin.to_rpc()))
        })();
        let draft_cleared = stored_origin.is_ok();
        let document = client.borrow().document.clone();
        let root = client.borrow().root.clone();
        let status = match root.query_selector(".hero-library-status")? {
            Some(status) => status,
            None => node(
                &document,
                &root,
                "p",
                "hero-library-status",
                "Not yet saved to your heroes",
            )?,
        };
        status.set_attribute("role", "status")?;
        if !draft_cleared {
            status.set_text_content(Some(
                "Your hero is accepted. This device could not clear its old draft.",
            ));
        }
        use sha2::{Digest, Sha256};
        let signature = format!(
            "{}|{}|{}|{:x}",
            client.borrow().credential,
            own.name,
            own.portrait_uri,
            Sha256::digest(
                own.identity
                    .as_ref()
                    .map(Message::encode_to_vec)
                    .unwrap_or_default()
            )
        );
        if LIBRARY_SAVED.with(|saved| *saved.borrow() == signature) {
            status.set_text_content(Some("Saved to your heroes"));
            return Ok(());
        }
        if LIBRARY_SAVING.with(|saving| saving.replace(true)) {
            return Ok(());
        }
        status.set_text_content(Some(
            "Not yet saved to your heroes - retrying when connected",
        ));
        let key = match library_key(true) {
            Ok(Some(key)) => key,
            _ => {
                LIBRARY_SAVING.with(|saving| saving.set(false));
                return Ok(());
            }
        };
        let state = client.borrow();
        let credential = state.credential.clone();
        let body = rpc::SaveMySeatHeroRequest {
            loaded_hero: match stored_origin {
                Ok(origin) => origin,
                Err(_) => {
                    LIBRARY_SAVING.with(|saving| saving.set(false));
                    status.set_text_content(Some("Not yet saved to your heroes - this device could not retain the hero reference"));
                    return Ok(());
                }
            },
            key,
            scope: Some(rpc::WatchViewRequest {
                session_id: state.session.clone(),
                run_id: state.run.clone(),
                client_binding_id: state.binding.clone(),
                after_revision: None,
            }),
        };
        drop(state);
        let weak = Rc::downgrade(client);
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let (connection, mut library) = connect_library().await?;
                let result = library
                    .save_my_seat_hero(library_request(body, &credential)?)
                    .await;
                connection.close();
                result.map_err(|status| {
                    library_failed(&weak, 4, status.code());
                    "Hero not yet saved".to_owned()
                })
            }
            .await;
            LIBRARY_SAVING.with(|saving| saving.set(false));
            let Some(client) = weak.upgrade() else {
                return;
            };
            if client.borrow().credential != credential {
                return;
            }
            if let Ok(response) = result {
                let saved = response.into_inner();
                let origin = draft_storage::SeatOrigin {
                    credential: credential.clone(),
                    loaded_hero: Some(draft_storage::LoadedHero {
                        hero_id: saved.hero_id,
                        revision: saved.revision,
                    }),
                };
                let retained = phone_storage().and_then(|storage| {
                    storage.set_item(
                        draft_storage::ORIGIN_KEY,
                        &origin
                            .encode()
                            .ok_or_else(|| JsValue::from_str("Hero reference invalid"))?,
                    )
                });
                if retained.is_err() {
                    status.set_text_content(Some(
                        "Saved to your heroes. This device could not retain the hero reference.",
                    ));
                    return;
                }
                LIBRARY_SAVED.with(|saved| *saved.borrow_mut() = signature);
                status.set_text_content(Some("Saved to your heroes"));
            } else {
                status.set_text_content(Some("Not yet saved to your heroes - reconnect to retry"));
            }
        });
        return Ok(());
    }
    let mut state = client.borrow_mut();
    let Some(creator) = state.creation.as_mut() else {
        return Ok(());
    };
    if creator.root.has_attribute("data-library-mounted") {
        return Ok(());
    }
    creator.root.set_attribute("data-library-mounted", "true")?;
    let document = creator
        .root
        .owner_document()
        .ok_or_else(|| JsValue::from_str("Document unavailable"))?;
    let picker = node(&document, &creator.root, "section", "hero-library", "")?;
    set_library_visibility(&picker, false)?;
    if let Some(layout) = creator.root.query_selector(".hero-layout")? {
        creator.root.insert_before(&picker, Some(&layout))?;
    }
    let heading = node(&document, &picker, "div", "hero-library-head", "")?;
    node(&document, &heading, "h2", "", "Your heroes")?;
    node(
        &document,
        &heading,
        "p",
        "hero-library-note",
        "Play a saved hero again, or begin a new one.",
    )?;
    let list = node(&document, &picker, "div", "hero-library-list", "")?;
    let new = node(&document, &list, "button", "hero-library-new", "")?;
    new.set_attribute("type", "button")?;
    node(&document, &new, "span", "hero-library-arch", "+")?
        .set_attribute("aria-hidden", "true")?;
    node(&document, &new, "strong", "", "New hero")?;
    mount_library_removal(creator, client, &document, &picker, &list)?;
    let defaults = creator.initial_draft.clone();
    let controls = creator.controls.clone();
    let chosen = list.clone();
    creator.listen(new, "click", move |_| {
        if controls.pending.get() {
            return;
        }
        mark_library_choice(&chosen, None);
        let mut draft = defaults.clone();
        draft.loaded_hero = None;
        draft.appearance_text.clear();
        draft.backstory_text.clear();
        draft.portrait_uri.clear();
        draft.portrait_revision = None;
        controls.draft_timer.borrow_mut().take();
        if controls.apply_stored(&draft).is_ok() {
            controls.portrait_version.set(None);
            controls.set_portrait_pending(false);
            controls.save_draft();
        }
    })?;
    let credential = state.credential.clone();
    drop(state);
    let key = match library_key(false) {
        Ok(Some(key)) => key,
        _ => return Ok(()),
    };
    let weak = Rc::downgrade(client);
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            let (connection, mut library) = connect_library().await?;
            let result = async {
                let started = web_time::Instant::now();
                let mut heroes = Vec::new();
                let mut cursor = 0;
                for _ in 0..100 {
                    let page = library
                        .list_my_heroes(library_request(
                            rpc::ListMyHeroesRequest {
                                key: key.clone(),
                                after_hero_id: cursor,
                            },
                            &credential,
                        )?)
                        .await
                        .map_err(|status| {
                            library_failed(&weak, 1, status.code());
                            "Your heroes are unavailable. You can still make a new hero.".to_owned()
                        })?
                        .into_inner();
                    heroes.extend(page.heroes);
                    if page.next_cursor == 0 || heroes.len() >= 100 {
                        return Ok(rpc::ListMyHeroesResponse {
                            heroes,
                            next_cursor: page.next_cursor,
                        });
                    }
                    if page.next_cursor <= cursor
                        || started.elapsed() > std::time::Duration::from_secs(20)
                    {
                        return Err(
                            "Your heroes took too long to load. Reconnect to retry.".to_owned()
                        );
                    }
                    cursor = page.next_cursor;
                }
                Err("Your heroes could not finish loading.".to_owned())
            }
            .await;
            connection.close();
            result
        }
        .await;
        let Some(client) = weak.upgrade() else {
            return;
        };
        let result = (|| -> Result<(), JsValue> {
            let rows = result.map_err(|message| JsValue::from_str(&message))?;
            let mut state = client.borrow_mut();
            let Some(creator) = state.creation.as_mut() else {
                return Ok(());
            };
            if !list.is_connected() {
                return Ok(());
            }
            if rows.heroes.is_empty() {
                set_library_visibility(&picker, false)?;
            } else {
                set_library_visibility(&picker, true)?;
            }
            for row in rows.heroes {
                let button = node(&document, &list, "button", "hero-library-item", "")?;
                button.set_attribute("type", "button")?;
                button.set_attribute("aria-pressed", "false")?;
                button.set_attribute("data-hero-id", &row.hero_id.to_string())?;
                button.set_attribute("data-hero-name", &row.name)?;
                let arch = node(&document, &button, "span", "hero-library-arch", "")?;
                if row.portrait.is_empty() {
                    let initial = row
                        .name
                        .chars()
                        .next()
                        .map(String::from)
                        .unwrap_or_default();
                    node(&document, &arch, "span", "hero-library-initial", &initial)?
                        .set_attribute("aria-hidden", "true")?;
                } else {
                    let image = node(&document, &arch, "img", "", "")?;
                    image.set_attribute("alt", "")?;
                    image.set_attribute("src", &portrait_data(&row)?)?;
                }
                node(&document, &button, "strong", "", &row.name)?;
                node(
                    &document,
                    &button,
                    "span",
                    "hero-library-meta",
                    &library_design_label(&row.species, &row.character_class),
                )?;
                let chosen = list.clone();
                let mark = button.clone();
                let weak = Rc::downgrade(&client);
                let key = key.clone();
                let credential = credential.clone();
                let hero_id = row.hero_id;
                creator.listen(button, "click", move |_| {
                    let weak = weak.clone();
                    let key = key.clone();
                    let credential = credential.clone();
                    let Some(owner)=weak.upgrade() else {return;};
                    let state=owner.borrow();let Some(creator)=state.creation.as_ref() else {return;};
                    if creator.controls.pending.get() {return;}
                    creator.set_pending(true);
                    creator.show_error("Loading your hero...");
                    let _ = mark.set_attribute("aria-busy", "true");
                    let chosen = chosen.clone();
                    let mark = mark.clone();
                    let version=Some(creator.controls.version.get());
                    drop(state);
                    wasm_bindgen_futures::spawn_local(async move {
                        let result = async {
                            let (connection, mut library) = connect_library().await?;
                            let result = library
                                .load_hero(library_request(
                                    rpc::LoadHeroRequest {
                                        key,
                                        hero_id,
                                        new_hero: false,
                                    },
                                    &credential,
                                )?)
                                .await;
                            connection.close();
                            result
                                .map(|response| response.into_inner())
                                .map_err(|status| { library_failed(&weak, 2, status.code()); "Hero could not load".to_owned() })
                        }
                        .await;
                        let Some(client) = weak.upgrade() else {
                            return;
                        };
                        let state = client.borrow();
                        let Some(creator) = state.creation.as_ref() else {
                            return;
                        };
                        creator.set_pending(false);
                        let _ = mark.remove_attribute("aria-busy");
                        if Some(creator.controls.version.get()) != version {
                            creator.show_error("Your newer edits are kept. Choose the saved hero again to replace them.");
                            return;
                        }
                        creator.show_error("");
                        match result {
                            Ok(loaded) => {
                                if let (Some(hero), Some(summary)) = (loaded.hero, loaded.summary) {
                                    if creator.apply_library(hero, summary).is_ok() {
                                        mark_library_choice(&chosen, Some(&mark));
                                    } else {
                                        creator.show_error(
                                            "Hero details could not load. Your draft is kept.",
                                        );
                                    }
                                }
                            }
                            Err(_) => creator
                                .show_error("Your heroes could not load. Your draft is kept."),
                        }
                    });
                })?;
            }
            Ok(())
        })();
        if result.is_err() && list.is_connected() {
            list.set_text_content(Some(
                "Your heroes could not load. You can still make a new hero.",
            ));
        }
    });
    Ok(())
}
