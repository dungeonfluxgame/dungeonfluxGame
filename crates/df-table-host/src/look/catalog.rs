//! The look studio as data: `<playground>/look/studio.json`, read fresh on every use so the owner
//! retunes a control, a species' height range or a prompt without a rebuild. A missing file is the
//! repository's copy built in. It holds the frames' figures, the image prompts, each species'
//! frame, height range and extra controls, and the controls themselves.
//!
//! Everything here is pure: the controls a species is shown, a draft checked against them, the
//! public description of a draft and the change list between two drafts (`planning/
//! phone-and-creation.md` P3, "Refine"). A hero's name and private story are never an input, so
//! they cannot reach an image prompt.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use df_playground::look::wire::{
    BodyFrame, ControlKind, ControlValue, LookControl, LookControls, LookDetail, LookDraft,
    LookOption, LookStartOptions, StartCard,
};
use serde::Deserialize;
use serde::de::IgnoredAny;

use super::request::{DraftCause, LookError, StudioCause};
use crate::adapters::EntropySource;
use crate::dev_http::BodyError;
use crate::imagegen::Aspect;
use crate::io_cause::IoCause;
use crate::seat::book::HeroRecord;

/// The folder of the studio and poses files under the playground's assets.
pub(crate) const LOOK_DIRECTORY: &str = "look";
const STUDIO_FILE: &str = "studio.json";
const BUILT_IN_STUDIO: &str = include_str!("../../../../assets/playground/look/studio.json");

/// The words of one frame's base figure.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameText {
    figure: String,
}

/// The image prompts. `{name}` marks where a value goes; nothing else is read from them.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Prompts {
    pose: String,
    pub(crate) pose_aspect: Aspect,
    pub(crate) sheet: String,
    pub(crate) sheet_aspect: Aspect,
    first: String,
    edit: String,
    pub(crate) pose_shows: String,
    pub(crate) lock_sheet: String,
    /// The ready picture: an edit of the portrait, `{pose}` the class's ready pose.
    ready: String,
    /// The square head-and-shoulders picture of a locked-in hero.
    pub(crate) headshot: String,
    pub(crate) headshot_aspect: Aspect,
    /// The half-body portrait of a locked-in hero.
    pub(crate) half_portrait: String,
    pub(crate) half_portrait_aspect: Aspect,
    context: String,
    detail: String,
    add: String,
    remove: String,
    change: String,
    /// The prompts the owner's example pictures were made with; the host does not read them.
    #[serde(rename = "example_species")]
    _example_species: IgnoredAny,
    #[serde(rename = "example_class")]
    _example_class: IgnoredAny,
}

/// The example picture of a species or a class on the start form, and its one line for the
/// player. `look` is the words the picture was made from; the host does not read them.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Example {
    picture: String,
    line: String,
    #[serde(rename = "look")]
    _look: IgnoredAny,
}

/// A class a new hero may be started as.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassData {
    id: String,
    label: String,
    example: Example,
    /// How a hero of the class stands ready, for the ready picture (`prompts.ready`).
    ready_pose: String,
}

impl Example {
    /// The start card of `id`, showing the example picture when `held` says the media library
    /// has it.
    fn card(&self, id: &str, label: &str, held: &impl Fn(&str) -> bool) -> StartCard {
        StartCard {
            id: id.to_owned(),
            label: label.to_owned(),
            line: self.line.clone(),
            picture: held(&self.picture).then(|| self.picture.clone()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpeciesLook {
    label: String,
    frame: BodyFrame,
    /// The shortest and tallest height of the species, in inches.
    height_in: [u32; 2],
    #[serde(default)]
    extras: Vec<String>,
    example: Example,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionData {
    id: String,
    label: String,
    /// The option in a picture's prompt.
    words: String,
    #[serde(default)]
    colour: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum KindData {
    /// A slider over the species' height range, in inches; its default is the middle.
    Height,
    /// A slider over `words`, from the first.
    Steps { words: Vec<String>, default: u32 },
    Swatch {
        options: Vec<OptionData>,
        default: String,
    },
    Chip {
        options: Vec<OptionData>,
        default: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlData {
    id: String,
    label: String,
    /// What the control changes, in a change list ("hair colour").
    noun: String,
    /// The control's value in the description ("{value} hair").
    phrase: String,
    /// Shown only to the species that name it among their extras.
    #[serde(default)]
    extra: bool,
    kind: KindData,
}

/// The studio file, read and checked.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Studio {
    frames: BTreeMap<BodyFrame, FrameText>,
    pub(crate) prompts: Prompts,
    /// The classes a new hero may be started as.
    classes: Vec<ClassData>,
    /// Common details a rolled hero may get one or two of.
    roll_details: Vec<LookDetail>,
    species: BTreeMap<String, SpeciesLook>,
    controls: Vec<ControlData>,
}

/// A control the player's words may set (`jev::read_details`): its id and label, and each of its
/// options as the reading names it, with the option's words. A steps control's options are the
/// indexes of its words ("0", "1", ...); a chip's or a swatch's are its option ids. The height
/// slider is not one: a number of inches is not read from words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WordControl {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) options: Vec<(String, String)>,
    steps: bool,
}

impl WordControl {
    /// The draft value of the option named `option`, when it is one of this control's.
    pub(crate) fn value_of(&self, option: &str) -> Option<ControlValue> {
        if !self.options.iter().any(|(key, _)| key == option) {
            return None;
        }
        Some(if self.steps {
            ControlValue::Number(option.parse().ok()?)
        } else {
            ControlValue::Choice(option.to_owned())
        })
    }
}

/// The hero a look is for, as public visual context: never a name or a private story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublicHero {
    pub(crate) species: String,
    pub(crate) class: String,
}

impl PublicHero {
    /// The public visual context of a hero of the library: its species and class, never its name.
    pub(crate) fn of(hero: &HeroRecord) -> Self {
        Self {
            species: hero.species.clone(),
            class: hero.class.clone(),
        }
    }
}

/// `template` with each `{name}` of `values` filled in, in one pass: a value is never read again
/// for a placeholder, so a player's words cannot fill one. An unknown `{name}` stays as written.
pub(crate) fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut filled = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        filled.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            filled.push_str(&rest[open..]);
            return filled;
        };
        let name = &after[..close];
        match values.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => filled.push_str(value),
            None => filled.push_str(&rest[open..open + close + 2]),
        }
        rest = &after[close + 1..];
    }
    filled.push_str(rest);
    filled
}

/// A height in inches as a person says it: `5 ft 4 in`, or `3 ft`.
fn feet_and_inches(inches: u32) -> String {
    match (inches / 12, inches % 12) {
        (feet, 0) => format!("{feet} ft"),
        (feet, rest) => format!("{feet} ft {rest} in"),
    }
}

fn option_words<'data>(options: &'data [OptionData], id: &str) -> Option<&'data str> {
    options
        .iter()
        .find(|option| option.id == id)
        .map(|option| option.words.as_str())
}

/// A number below `count` from `entropy` (none from no choices). The modulo bias is below one in
/// 2^58 for the few choices a control has.
fn pick(count: usize, entropy: &mut dyn EntropySource) -> usize {
    let count = u64::try_from(count).unwrap_or(u64::MAX).max(1);
    usize::try_from(entropy.next_u64() % count).unwrap_or(0)
}

impl ControlData {
    /// A value of the control at random.
    fn random(&self, height_in: [u32; 2], entropy: &mut dyn EntropySource) -> ControlValue {
        let number = |index: usize| u32::try_from(index).unwrap_or(0);
        match &self.kind {
            KindData::Height => {
                let range = usize::try_from(height_in[1] - height_in[0]).unwrap_or(0);
                ControlValue::Number(height_in[0] + number(pick(range + 1, entropy)))
            }
            KindData::Steps { words, .. } => {
                ControlValue::Number(number(pick(words.len(), entropy)))
            }
            KindData::Swatch { options, default } | KindData::Chip { options, default } => {
                let chosen = options
                    .get(pick(options.len(), entropy))
                    .map_or(default, |option| &option.id);
                ControlValue::Choice(chosen.clone())
            }
        }
    }

    fn check(&self) -> Result<(), StudioCause> {
        let invalid = || StudioCause::InvalidControl {
            control: self.id.clone(),
        };
        let fits = match &self.kind {
            KindData::Height => true,
            KindData::Steps { words, default } => {
                usize::try_from(*default).is_ok_and(|default| default < words.len())
            }
            KindData::Swatch { options, default } | KindData::Chip { options, default } => {
                let ids: BTreeSet<&str> = options.iter().map(|option| option.id.as_str()).collect();
                ids.len() == options.len() && ids.contains(default.as_str())
            }
        };
        if fits { Ok(()) } else { Err(invalid()) }
    }

    fn wire(&self, height_in: [u32; 2]) -> LookControl {
        let wire_options = |options: &[OptionData]| -> Vec<LookOption> {
            options
                .iter()
                .map(|option| LookOption {
                    id: option.id.clone(),
                    label: option.label.clone(),
                    colour: option.colour.clone(),
                })
                .collect()
        };
        let (kind, default) = match &self.kind {
            KindData::Height => (
                ControlKind::Slider {
                    min: height_in[0],
                    max: height_in[1],
                    unit: Some("in".to_owned()),
                    words: Vec::new(),
                },
                ControlValue::Number(height_in[0] + (height_in[1] - height_in[0]) / 2),
            ),
            KindData::Steps { words, default } => (
                ControlKind::Slider {
                    min: 0,
                    max: u32::try_from(words.len().saturating_sub(1)).unwrap_or(u32::MAX),
                    unit: None,
                    words: words.clone(),
                },
                ControlValue::Number(*default),
            ),
            KindData::Swatch { options, default } => (
                ControlKind::Swatch {
                    options: wire_options(options),
                },
                ControlValue::Choice(default.clone()),
            ),
            KindData::Chip { options, default } => (
                ControlKind::Chip {
                    options: wire_options(options),
                },
                ControlValue::Choice(default.clone()),
            ),
        };
        LookControl {
            id: self.id.clone(),
            label: self.label.clone(),
            kind,
            default,
        }
    }

    /// `value` in a prompt's words, when the control takes it.
    fn words(&self, height_in: [u32; 2], value: &ControlValue) -> Option<String> {
        match (&self.kind, value) {
            (KindData::Height, ControlValue::Number(inches))
                if (height_in[0]..=height_in[1]).contains(inches) =>
            {
                Some(feet_and_inches(*inches))
            }
            (KindData::Steps { words, .. }, ControlValue::Number(step)) => usize::try_from(*step)
                .ok()
                .and_then(|step| words.get(step))
                .cloned(),
            (
                KindData::Swatch { options, .. } | KindData::Chip { options, .. },
                ControlValue::Choice(id),
            ) => option_words(options, id).map(str::to_owned),
            _ => None,
        }
    }
}

impl Studio {
    /// Checks what serde cannot: every species' frame has a figure, control ids are unique, each
    /// extra a species names is an extra control, and every default is one of its control's values.
    fn check(&self) -> Result<(), StudioCause> {
        let mut ids = BTreeSet::new();
        for control in &self.controls {
            if !ids.insert(control.id.as_str()) {
                return Err(StudioCause::DuplicateControl {
                    control: control.id.clone(),
                });
            }
            control.check()?;
        }
        for (key, species) in &self.species {
            if !self.frames.contains_key(&species.frame) {
                return Err(StudioCause::MissingFrame {
                    frame: species.frame,
                });
            }
            if species.height_in[0] > species.height_in[1] {
                return Err(StudioCause::InvalidHeight {
                    species: key.clone(),
                });
            }
            if let Some(unknown) = species.extras.iter().find(|extra| {
                !self
                    .controls
                    .iter()
                    .any(|control| control.extra && &control.id == *extra)
            }) {
                return Err(StudioCause::UnknownExtra {
                    species: key.clone(),
                    control: unknown.clone(),
                });
            }
        }
        Ok(())
    }

    fn species(&self, species: &str) -> Result<&SpeciesLook, LookError> {
        self.species
            .get(species)
            .ok_or_else(|| LookError::UnknownSpecies {
                species: species.to_owned(),
            })
    }

    /// What a new hero may be started as, each with its example picture when `held` says the
    /// media library has it.
    pub(crate) fn start_options(&self, held: impl Fn(&str) -> bool) -> LookStartOptions {
        LookStartOptions {
            species: self
                .species
                .iter()
                .map(|(id, look)| look.example.card(id, &look.label, &held))
                .collect(),
            classes: self
                .classes
                .iter()
                .map(|class| class.example.card(&class.id, &class.label, &held))
                .collect(),
        }
    }

    /// Every example picture the start form names.
    pub(crate) fn example_pictures(&self) -> Vec<String> {
        self.species
            .values()
            .map(|look| look.example.picture.clone())
            .chain(
                self.classes
                    .iter()
                    .map(|class| class.example.picture.clone()),
            )
            .collect()
    }

    /// Whether `species` and `class` are ones a hero may be started as.
    /// Whether `detail` is one of the studio's rolled details.
    pub(crate) fn rolls(&self, detail: &LookDetail) -> bool {
        self.roll_details.contains(detail)
    }

    /// The prompt of the ready picture of a hero of `class`.
    pub(crate) fn ready_prompt(&self, class: &str) -> Result<String, LookError> {
        let found = self
            .classes
            .iter()
            .find(|known| known.id == class)
            .ok_or_else(|| LookError::UnknownClass {
                class: class.to_owned(),
            })?;
        Ok(fill(&self.prompts.ready, &[("pose", &found.ready_pose)]))
    }

    pub(crate) fn check_start(&self, species: &str, class: &str) -> Result<(), LookError> {
        self.species(species)?;
        if self.classes.iter().any(|known| known.id == class) {
            Ok(())
        } else {
            Err(LookError::UnknownClass {
                class: class.to_owned(),
            })
        }
    }

    /// The frame `species` stands on.
    pub(crate) fn frame_of(&self, species: &str) -> Result<BodyFrame, LookError> {
        Ok(self.species(species)?.frame)
    }

    /// The controls `species` is shown, in the file's order: every hero's, then its own extras.
    fn controls_of<'studio>(
        &'studio self,
        species: &'studio SpeciesLook,
    ) -> impl Iterator<Item = &'studio ControlData> {
        self.controls
            .iter()
            .filter(|control| !control.extra || species.extras.contains(&control.id))
    }

    /// The controls of `species` the player's words may set, in the file's order.
    pub(crate) fn word_controls(&self, species: &str) -> Result<Vec<WordControl>, LookError> {
        let look = self.species(species)?;
        Ok(self
            .controls_of(look)
            .filter_map(|control| {
                let (options, steps) = match &control.kind {
                    KindData::Height => return None,
                    KindData::Steps { words, .. } => (
                        words
                            .iter()
                            .enumerate()
                            .map(|(index, words)| (index.to_string(), words.clone()))
                            .collect(),
                        true,
                    ),
                    KindData::Swatch { options, .. } | KindData::Chip { options, .. } => (
                        options
                            .iter()
                            .map(|option| (option.id.clone(), option.words.clone()))
                            .collect(),
                        false,
                    ),
                };
                Some(WordControl {
                    id: control.id.clone(),
                    label: control.label.clone(),
                    options,
                    steps,
                })
            })
            .collect())
    }

    /// The controls the phone draws for `hero`, of `species`.
    pub(crate) fn controls(&self, species: &str, hero: u64) -> Result<LookControls, LookError> {
        let look = self.species(species)?;
        Ok(LookControls {
            hero,
            frame: look.frame,
            controls: self
                .controls_of(look)
                .map(|control| control.wire(look.height_in))
                .collect(),
        })
    }

    /// `draft` checked against the controls of `species`, with every value it leaves out at its
    /// default and every detail's words trimmed.
    /// `draft` with every control it leaves out set at random within the control's values, and,
    /// with `details`, one or two of the common details in place of its own (a roll). What it
    /// sets stays.
    pub(crate) fn roll(
        &self,
        species: &str,
        draft: &LookDraft,
        details: bool,
        entropy: &mut dyn EntropySource,
    ) -> Result<LookDraft, LookError> {
        let look = self.species(species)?;
        let mut rolled = draft.clone();
        for control in self.controls_of(look) {
            if !rolled.values.contains_key(&control.id) {
                let value = control.random(look.height_in, entropy);
                rolled.values.insert(control.id.clone(), value);
            }
        }
        if details {
            rolled.details.clear();
            for _ in 0..=pick(2, entropy) {
                if let Some(detail) = self
                    .roll_details
                    .get(pick(self.roll_details.len(), entropy))
                    && !rolled.details.contains(detail)
                {
                    rolled.details.push(detail.clone());
                }
            }
        }
        Ok(rolled)
    }

    pub(crate) fn settle(&self, species: &str, draft: &LookDraft) -> Result<LookDraft, LookError> {
        let look = self.species(species)?;
        let invalid = |cause| LookError::InvalidDraft { cause };
        if let Some(unknown) = draft
            .values
            .keys()
            .find(|id| !self.controls_of(look).any(|control| &control.id == *id))
        {
            return Err(invalid(DraftCause::UnknownControl {
                control: unknown.clone(),
            }));
        }
        let mut values = BTreeMap::new();
        for control in self.controls_of(look) {
            let value = draft
                .values
                .get(&control.id)
                .cloned()
                .unwrap_or_else(|| control.wire(look.height_in).default);
            if control.words(look.height_in, &value).is_none() {
                return Err(invalid(DraftCause::InvalidValue {
                    control: control.id.clone(),
                }));
            }
            values.insert(control.id.clone(), value);
        }
        let mut details = Vec::with_capacity(draft.details.len());
        for detail in &draft.details {
            let what = detail.what.trim();
            if what.is_empty() {
                return Err(invalid(DraftCause::EmptyDetail));
            }
            details.push(LookDetail {
                what: what.to_owned(),
                ..detail.clone()
            });
        }
        Ok(LookDraft { values, details })
    }

    fn detail_words(&self, detail: &LookDetail) -> String {
        fill(
            &self.prompts.detail,
            &[
                ("what", &detail.what),
                ("kind", detail.kind.words()),
                ("region", detail.place.words()),
            ],
        )
    }

    /// Each control's value of a settled `draft` in a prompt's words, in the controls' order.
    fn value_phrases(&self, look: &SpeciesLook, draft: &LookDraft) -> Vec<String> {
        self.controls_of(look)
            .filter_map(|control| {
                let value = draft.values.get(&control.id)?;
                let words = control.words(look.height_in, value)?;
                Some(fill(&control.phrase, &[("value", &words)]))
            })
            .collect()
    }

    /// The hero's species and class as public visual context.
    pub(crate) fn hero_context(&self, hero: &PublicHero) -> String {
        fill(
            &self.prompts.context,
            &[("species", &hero.species), ("class", &hero.class)],
        )
    }

    /// The whole public look of `hero` with a settled `draft`: species and class, each control's
    /// value, then each detail.
    pub(crate) fn description(
        &self,
        hero: &PublicHero,
        draft: &LookDraft,
    ) -> Result<String, LookError> {
        let look = self.species(&hero.species)?;
        let mut parts = vec![self.hero_context(hero)];
        parts.extend(self.value_phrases(look, draft));
        parts.extend(draft.details.iter().map(|detail| self.detail_words(detail)));
        Ok(parts.join("; "))
    }

    /// The details of `draft` in a prompt's words (the chips: an aura, a scar, jewellery), joined;
    /// none when it has none.
    pub(crate) fn details_text(&self, draft: &LookDraft) -> Option<String> {
        let words: Vec<String> = draft
            .details
            .iter()
            .map(|detail| self.detail_words(detail))
            .collect();
        (!words.is_empty()).then(|| words.join("; "))
    }

    /// What changed from the settled draft `before` to the settled draft `after`, as the edit asks
    /// for it: each control whose value changed, each detail removed, then each detail added.
    pub(crate) fn changes(
        &self,
        species: &str,
        before: &LookDraft,
        after: &LookDraft,
    ) -> Result<Vec<String>, LookError> {
        let look = self.species(species)?;
        let mut changes = Vec::new();
        for control in self.controls_of(look) {
            let (Some(old), Some(new)) = (
                before.values.get(&control.id),
                after.values.get(&control.id),
            ) else {
                continue;
            };
            if old == new {
                continue;
            }
            let (Some(old), Some(new)) = (
                control.words(look.height_in, old),
                control.words(look.height_in, new),
            ) else {
                continue;
            };
            changes.push(fill(
                &self.prompts.change,
                &[("noun", &control.noun), ("old", &old), ("new", &new)],
            ));
        }
        for removed in before
            .details
            .iter()
            .filter(|detail| !after.details.contains(detail))
        {
            let detail = self.detail_words(removed);
            changes.push(fill(&self.prompts.remove, &[("detail", &detail)]));
        }
        for added in after
            .details
            .iter()
            .filter(|detail| !before.details.contains(detail))
        {
            let detail = self.detail_words(added);
            changes.push(fill(&self.prompts.add, &[("detail", &detail)]));
        }
        Ok(changes)
    }

    /// The prompt of a frame's base pose.
    pub(crate) fn pose_prompt(&self, frame: BodyFrame) -> String {
        let figure = self
            .frames
            .get(&frame)
            .map_or("", |text| text.figure.as_str());
        fill(&self.prompts.pose, &[("figure", figure)])
    }

    /// The prompt of a version rendered from the base pose with the whole description.
    pub(crate) fn first_prompt(&self, description: &str) -> String {
        fill(&self.prompts.first, &[("description", description)])
    }

    /// The prompt of a version that edits the kept one: only what changed.
    pub(crate) fn edit_prompt(&self, changes: &[String]) -> String {
        fill(&self.prompts.edit, &[("changes", &changes.join("; "))])
    }
}

/// The studio read from `text` and checked.
pub(crate) fn parse_studio(text: &str) -> Result<Studio, LookError> {
    let studio: Studio = serde_json::from_str(text).map_err(|error| LookError::StudioInvalid {
        cause: BodyError::from_json(&error),
    })?;
    studio
        .check()
        .map_err(|cause| LookError::StudioRejected { cause })?;
    Ok(studio)
}

/// The studio file under the playground's `assets`, or the built-in one when there is none.
/// Blocking.
pub(crate) fn read_studio(assets: &Path) -> Result<Studio, LookError> {
    let path = assets.join(LOOK_DIRECTORY).join(STUDIO_FILE);
    match std::fs::read_to_string(path) {
        Ok(text) => parse_studio(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => parse_studio(BUILT_IN_STUDIO),
        Err(error) => Err(LookError::StudioUnreadable {
            cause: IoCause::from_kind(error.kind()),
        }),
    }
}

#[cfg(test)]
pub(crate) fn built_in_studio() -> Studio {
    parse_studio(BUILT_IN_STUDIO).unwrap_or_else(|error| panic!("{error}"))
}
