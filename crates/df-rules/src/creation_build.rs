//! Creation options and every-reason build validation for the selected SRD 5.2.1 creation subset.
//! Owns: `creation_options`, the typed listing of every choice a creator may make, read from the
//! existing constants and the rules' ability arrays; and `validate_build`, which accepts exactly
//! the builds `local_journey::validate_character` accepts and otherwise lists every rule the build
//! breaks. The first reason of a rejection carries the `JourneyRuleError` that `validate_character`
//! returns. Never does: admit a build, add a choice, change a rule or a constant, read content
//! packs, or panic. Fails by typed value only. Invalid creation rules are reported alone, because
//! the ability arrays and background bonus that the other checks read cannot be trusted; a refusal
//! that names no rule gets one reason that says so, rather than an empty list.
//! First release: class Fighter 1 and background Soldier only (SRD pp.47, 83).
use crate::local_journey::{
    ALIGNMENTS, AbilityArray, BACKGROUND_OFFER, BUILD_GROUPS, CLASS_OFFER, CreationRules,
    EQUIPMENT_OFFER, FIGHTER_SKILLS, GAMING_SETS, HeroSpecies, JourneyRuleError, LegalCharacter,
    MAXIMUM_CHARACTER_NAME_CHARACTERS, SOLDIER_SKILLS, STANDARD_LANGUAGES, STYLE_OFFER, TIES_OFFER,
    valid_character_name, validate_character,
};
use crate::species::{SpeciesChoiceGroup, choice_groups, is_choice_group};
use serde::{Deserialize, Serialize};

/// The fixed single-option picks, as (choice key, the one value, plain noun, SRD citation).
const FIXED_PICKS: [(&str, &str, &str, &str); 5] = [
    ("class", CLASS_OFFER, "class", " (SRD p.47)"),
    ("background", BACKGROUND_OFFER, "background", " (SRD p.83)"),
    (
        "equipment",
        EQUIPMENT_OFFER,
        "starting equipment",
        " (SRD p.47)",
    ),
    (
        "style-masteries",
        STYLE_OFFER,
        "fighting style and weapon masteries",
        " (SRD p.88)",
    ),
    ("allied-ties", TIES_OFFER, "tie order", ""),
];

/// Plain noun for each build choice key, used when a pick is missing.
const BUILD_SLOTS: [(&str, &str); 13] = [
    ("species", "species"),
    ("class", "class"),
    ("background", "background"),
    ("array", "ability scores"),
    ("alignment", "alignment"),
    ("language-1", "first language"),
    ("language-2", "second language"),
    ("skill-1", "first skill"),
    ("skill-2", "second skill"),
    ("gaming-set", "gaming set"),
    ("equipment", "starting equipment"),
    ("style-masteries", "fighting style and weapon masteries"),
    ("allied-ties", "tie order"),
];

/// Labels that a plain rule of capitalising each hyphen-separated word would get wrong.
const LABEL_OVERRIDES: [(&str, &str); 4] = [
    ("fighter-1", "Fighter, level 1"),
    (
        "fighter-a-soldier-a",
        "Chain mail, greatsword, flail, 8 javelins",
    ),
    ("defense-greatsword-flail-javelin", "Defense"),
    ("join-order", "Party goes first"),
];

/// A typed listing of every choice a creator may make, grouped as the creator sees them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreationOptions {
    /// Build-wide groups, in the order a creator meets them. The species group lists the species
    /// ids.
    pub groups: Vec<OptionGroup>,
    /// Each species with the choice groups only that species asks for (SRD pp.84-86).
    pub species: Vec<SpeciesOffer>,
}

/// One group on the creation screen. `slots` are the build choice keys its picks fill, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionGroup {
    pub id: String,
    pub label: String,
    pub slots: Vec<String>,
    pub options: Vec<CreationOption>,
}

/// One pickable value: the id the build request sends and the plain label a creator reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreationOption {
    pub id: String,
    pub label: String,
}

/// A species and the species-only groups that its picks must fill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeciesOffer {
    pub id: String,
    pub label: String,
    pub groups: Vec<OptionGroup>,
}

/// One refused rule of a build. `error` is the same typed error `validate_character` returns for
/// it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BuildRejection {
    #[serde(serialize_with = "serialize_error")]
    pub error: JourneyRuleError,
    /// The choice key the reason is about; `None` for the name, the creation rules and the pick
    /// count.
    pub group: Option<String>,
    /// The value the creator picked that is refused, when a value was picked.
    pub option_id: Option<String>,
    /// One plain sentence for the creator, naming the offending value and any SRD rule.
    pub sentence: String,
}

/// The result of checking a build: the admitted character, or every refused rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildReport {
    Accepted(LegalCharacter),
    Rejected { reasons: Vec<BuildRejection> },
}

/// Every choice a creator may make, read from the existing creation constants.
pub fn creation_options(rules: &CreationRules<'_>) -> CreationOptions {
    let groups = Group::ALL
        .iter()
        .map(|group| OptionGroup {
            id: group.id().to_owned(),
            label: group.label().to_owned(),
            slots: group
                .slots()
                .iter()
                .map(|slot| (*slot).to_owned())
                .collect(),
            options: group
                .options(rules)
                .into_iter()
                .map(|id| CreationOption {
                    id: id.to_owned(),
                    label: option_label(id),
                })
                .collect(),
        })
        .collect();
    let species = HeroSpecies::ALL
        .iter()
        .map(|species| SpeciesOffer {
            id: species.choice_id().to_owned(),
            label: option_label(species.choice_id()),
            groups: choice_groups(*species).iter().map(species_group).collect(),
        })
        .collect();
    CreationOptions { groups, species }
}

/// Check a build. Accepted exactly when `validate_character` returns `Ok`, with the same character.
/// Rejected otherwise, with every refused rule in `validate_character`'s check order, so that the
/// first reason carries the error `validate_character` returns.
pub fn validate_build(
    rules: &CreationRules<'_>,
    name: &str,
    choices: &[(&str, &str)],
) -> BuildReport {
    match validate_character(rules, name, choices) {
        Ok(character) => BuildReport::Accepted(character),
        Err(error) => {
            let mut reasons = rejections(rules, name, choices);
            if reasons.is_empty() {
                reasons.push(BuildRejection {
                    error,
                    group: None,
                    option_id: None,
                    sentence: "The build was refused, but no single rule could be named for it."
                        .to_owned(),
                });
            }
            BuildReport::Rejected { reasons }
        }
    }
}

/// Serialise a refused error by its variant name, so the reason list is stable JSON.
fn serialize_error<S>(error: &JourneyRuleError, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&format!("{error:?}"))
}

/// The build groups in the order a creator meets them. Each group owns the choice keys in `slots`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Species,
    Class,
    Background,
    Array,
    Skills,
    Languages,
    Alignment,
    GamingSet,
    Equipment,
    StyleMasteries,
    AlliedTies,
}

impl Group {
    const ALL: [Group; 11] = [
        Group::Species,
        Group::Class,
        Group::Background,
        Group::Array,
        Group::Skills,
        Group::Languages,
        Group::Alignment,
        Group::GamingSet,
        Group::Equipment,
        Group::StyleMasteries,
        Group::AlliedTies,
    ];

    const fn id(self) -> &'static str {
        match self {
            Group::Species => "species",
            Group::Class => "class",
            Group::Background => "background",
            Group::Array => "array",
            Group::Skills => "skills",
            Group::Languages => "languages",
            Group::Alignment => "alignment",
            Group::GamingSet => "gaming-set",
            Group::Equipment => "equipment",
            Group::StyleMasteries => "style-masteries",
            Group::AlliedTies => "allied-ties",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Group::Species => "Species",
            Group::Class => "Class",
            Group::Background => "Background",
            Group::Array => "Ability scores",
            Group::Skills => "Skills (pick two)",
            Group::Languages => "Languages (pick two)",
            Group::Alignment => "Alignment",
            Group::GamingSet => "Gaming set",
            Group::Equipment => "Starting equipment",
            Group::StyleMasteries => "Fighting style and weapon masteries",
            Group::AlliedTies => "When turns are tied",
        }
    }

    const fn slots(self) -> &'static [&'static str] {
        match self {
            Group::Species => &["species"],
            Group::Class => &["class"],
            Group::Background => &["background"],
            Group::Array => &["array"],
            Group::Skills => &["skill-1", "skill-2"],
            Group::Languages => &["language-1", "language-2"],
            Group::Alignment => &["alignment"],
            Group::GamingSet => &["gaming-set"],
            Group::Equipment => &["equipment"],
            Group::StyleMasteries => &["style-masteries"],
            Group::AlliedTies => &["allied-ties"],
        }
    }

    fn options<'r>(self, rules: &CreationRules<'r>) -> Vec<&'r str> {
        match self {
            Group::Species => HeroSpecies::ALL
                .iter()
                .map(|species| species.choice_id())
                .collect(),
            Group::Class => vec![CLASS_OFFER],
            Group::Background => vec![BACKGROUND_OFFER],
            Group::Array => rules
                .ability_arrays
                .iter()
                .map(|array: &AbilityArray<'r>| array.id)
                .collect(),
            Group::Skills => FIGHTER_SKILLS.to_vec(),
            Group::Languages => STANDARD_LANGUAGES.to_vec(),
            Group::Alignment => ALIGNMENTS.to_vec(),
            Group::GamingSet => GAMING_SETS.to_vec(),
            Group::Equipment => vec![EQUIPMENT_OFFER],
            Group::StyleMasteries => vec![STYLE_OFFER],
            Group::AlliedTies => vec![TIES_OFFER],
        }
    }
}

/// A species choice group as a listing group. Soldier-granted skills are not offered again.
fn species_group(group: &SpeciesChoiceGroup) -> OptionGroup {
    let options = group
        .options
        .iter()
        .filter(|&&option| !(group.id == "human-skill" && SOLDIER_SKILLS.contains(&option)))
        .map(|&option| CreationOption {
            id: option.to_owned(),
            label: option_label(option),
        })
        .collect();
    OptionGroup {
        id: group.id.to_owned(),
        label: group.label.to_owned(),
        slots: vec![group.id.to_owned()],
        options,
    }
}

/// The plain label for an option id: its override, else its hyphen-separated words capitalised.
fn option_label(id: &str) -> String {
    if let Some((_, label)) = LABEL_OVERRIDES.iter().find(|(key, _)| *key == id) {
        return (*label).to_owned();
    }
    id.split('-')
        .enumerate()
        .map(|(index, word)| {
            if index > 0 && word == "of" {
                word.to_owned()
            } else {
                capitalise(word)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalise(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// Collects refused rules in check order. Identical reasons are kept once.
struct Collector<'a> {
    choices: &'a [(&'a str, &'a str)],
    reasons: Vec<BuildRejection>,
    missing: Vec<&'static str>,
}

impl<'a> Collector<'a> {
    /// The first value picked for a key, the one `validate_character` reads.
    fn value(&self, key: &str) -> Option<&'a str> {
        self.choices
            .iter()
            .find(|(picked, _)| *picked == key)
            .map(|(_, value)| *value)
    }

    fn has(&self, key: &str) -> bool {
        self.choices.iter().any(|(picked, _)| *picked == key)
    }

    fn push(
        &mut self,
        error: JourneyRuleError,
        group: Option<&str>,
        option_id: Option<&str>,
        sentence: String,
    ) {
        let repeated = self.reasons.iter().any(|reason| {
            reason.error == error
                && reason.group.as_deref() == group
                && reason.option_id.as_deref() == option_id
        });
        if !repeated {
            self.reasons.push(BuildRejection {
                error,
                group: group.map(str::to_owned),
                option_id: option_id.map(str::to_owned),
                sentence,
            });
        }
    }

    /// A missing pick is reported once per key, however many checks would notice it.
    fn missing(&mut self, group: &'static str, noun: &str) {
        if self.missing.contains(&group) {
            return;
        }
        self.missing.push(group);
        self.push(
            JourneyRuleError::MissingChoice,
            Some(group),
            None,
            format!("No {noun} was picked."),
        );
    }
}

/// Every refused rule, in the order `validate_character` checks them.
fn rejections(
    rules: &CreationRules<'_>,
    name: &str,
    choices: &[(&str, &str)],
) -> Vec<BuildRejection> {
    if rules.validate().is_err() {
        return vec![BuildRejection {
            error: JourneyRuleError::InvalidRules,
            group: None,
            option_id: None,
            sentence: "The creation rules are not valid, so no build can be judged against them."
                .to_owned(),
        }];
    }
    let mut collector = Collector {
        choices,
        reasons: Vec::new(),
        missing: Vec::new(),
    };
    if !valid_character_name(name) {
        collector.push(
            JourneyRuleError::InvalidName,
            None,
            None,
            format!(
                "The hero name must be 1 to {MAXIMUM_CHARACTER_NAME_CHARACTERS} characters: \
                    letters, joined by single spaces, apostrophes, hyphens, periods or \
                    underscores, starting with a letter and ending with a letter or period."
            ),
        );
    }
    if choices.len() < BUILD_GROUPS.len() {
        for (group, noun) in BUILD_SLOTS {
            if !collector.has(group) {
                collector.missing(group, noun);
            }
        }
    }
    for (index, &(group, _)) in choices.iter().enumerate() {
        if !BUILD_GROUPS.contains(&group) && !is_choice_group(group) {
            collector.push(
                JourneyRuleError::InvalidChoice,
                Some(group),
                None,
                format!("\"{group}\" is not a choice this build offers."),
            );
        }
        if choices[..index]
            .iter()
            .any(|(earlier, _)| *earlier == group)
        {
            collector.push(
                JourneyRuleError::DuplicateChoice,
                Some(group),
                None,
                format!("\"{group}\" was picked more than once; only one pick is allowed."),
            );
        }
    }
    species_pick(&mut collector);
    fixed_picks(&mut collector);
    array_pick(&mut collector, rules);
    single_pick(
        &mut collector,
        "alignment",
        ALIGNMENTS,
        "alignment",
        " (SRD p.21)",
    );
    single_pick(&mut collector, "gaming-set", GAMING_SETS, "gaming set", "");
    pair_picks(
        &mut collector,
        ("language-1", "language-2"),
        STANDARD_LANGUAGES,
        ("first language", "second language"),
        " (SRD p.20)",
    );
    pair_picks(
        &mut collector,
        ("skill-1", "skill-2"),
        FIGHTER_SKILLS,
        ("first skill", "second skill"),
        " (SRD p.47)",
    );
    collector.reasons
}

/// The species pick, then the choice groups that species asks for and the skill rules they share.
fn species_pick(collector: &mut Collector<'_>) {
    let species = match collector.value("species") {
        None => {
            collector.missing("species", "species");
            None
        }
        Some(value) => match HeroSpecies::from_choice(value) {
            Some(species) => Some(species),
            None => {
                collector.push(
                    JourneyRuleError::InvalidChoice,
                    Some("species"),
                    Some(value),
                    format!("\"{value}\" is not an offered species."),
                );
                None
            }
        },
    };
    if let Some(species) = species {
        species_groups(collector, species);
    }
}

fn species_groups(collector: &mut Collector<'_>, species: HeroSpecies) {
    let groups = choice_groups(species);
    for group in &groups {
        match collector.value(group.id) {
            None => collector.missing(group.id, group.label),
            Some(value) if !group.options.contains(&value) => collector.push(
                JourneyRuleError::InvalidChoice,
                Some(group.id),
                Some(value),
                format!("\"{value}\" is not an option for {}.", group.label),
            ),
            Some(_) => {}
        }
    }
    let choices = collector.choices;
    for &(key, _) in choices {
        if is_choice_group(key) && !groups.iter().any(|group| group.id == key) {
            collector.push(
                JourneyRuleError::InvalidChoice,
                Some(key),
                None,
                format!("\"{key}\" does not belong to the chosen species."),
            );
        }
    }
    let skill = collector
        .value("human-skill")
        .map(|value| ("human-skill", value))
        .or_else(|| {
            collector
                .value("keen-senses")
                .map(|value| ("keen-senses", value))
        });
    if let Some((key, skill)) = skill {
        if SOLDIER_SKILLS.contains(&skill) {
            collector.push(
                JourneyRuleError::InvalidChoice,
                Some(key),
                Some(skill),
                format!(
                    "{skill} is already granted by the Soldier background (SRD p.83), so it cannot \
                        be picked again."
                ),
            );
        } else if collector.value("skill-1") == Some(skill)
            || collector.value("skill-2") == Some(skill)
        {
            collector.push(
                JourneyRuleError::InvalidChoice,
                Some(key),
                Some(skill),
                format!("{skill} is already one of the two skill picks; choose a different skill."),
            );
        }
    }
}

/// The five single-option picks of the first release.
fn fixed_picks(collector: &mut Collector<'_>) {
    for (group, expected, noun, citation) in FIXED_PICKS {
        match collector.value(group) {
            None => collector.missing(group, noun),
            Some(value) if value != expected => collector.push(
                JourneyRuleError::InvalidChoice,
                Some(group),
                Some(value),
                format!(
                    "The {noun} must be \"{expected}\" in this first release{citation}, not \
                        \"{value}\"."
                ),
            ),
            Some(_) => {}
        }
    }
}

fn array_pick(collector: &mut Collector<'_>, rules: &CreationRules<'_>) {
    match collector.value("array") {
        None => collector.missing("array", "ability scores"),
        Some(value) if !rules.ability_arrays.iter().any(|array| array.id == value) => collector
            .push(
                JourneyRuleError::InvalidChoice,
                Some("array"),
                Some(value),
                format!("\"{value}\" is not one of the ability score arrays offered."),
            ),
        Some(_) => {}
    }
}

fn single_pick(
    collector: &mut Collector<'_>,
    key: &'static str,
    options: &[&str],
    noun: &str,
    citation: &str,
) {
    match collector.value(key) {
        None => collector.missing(key, noun),
        Some(value) if !options.contains(&value) => collector.push(
            JourneyRuleError::InvalidChoice,
            Some(key),
            Some(value),
            format!("\"{value}\" is not an offered {noun}{citation}."),
        ),
        Some(_) => {}
    }
}

/// Two picks from one list: each must be offered, and the two must differ.
fn pair_picks(
    collector: &mut Collector<'_>,
    (first, second): (&'static str, &'static str),
    options: &[&str],
    (first_noun, second_noun): (&str, &str),
    citation: &str,
) {
    let left = collector.value(first);
    let right = collector.value(second);
    if left.is_none() {
        collector.missing(first, first_noun);
    }
    if right.is_none() {
        collector.missing(second, second_noun);
    }
    if let Some(value) = left.filter(|value| !options.contains(value)) {
        collector.push(
            JourneyRuleError::InvalidChoice,
            Some(first),
            Some(value),
            format!("\"{value}\" is not an offered {first_noun}{citation}."),
        );
    }
    if let Some(value) = right.filter(|value| !options.contains(value)) {
        collector.push(
            JourneyRuleError::InvalidChoice,
            Some(second),
            Some(value),
            format!("\"{value}\" is not an offered {second_noun}{citation}."),
        );
    }
    if let (Some(left), Some(right)) = (left, right)
        && left == right
        && options.contains(&left)
    {
        collector.push(
            JourneyRuleError::InvalidChoice,
            Some(second),
            Some(right),
            format!(
                "The {second_noun} repeats the {first_noun}; pick two different ones{citation}."
            ),
        );
    }
}

#[cfg(test)]
#[path = "creation_build_tests.rs"]
mod tests;
