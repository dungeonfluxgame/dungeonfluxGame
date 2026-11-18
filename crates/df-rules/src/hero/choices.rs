//! What a hero build asks for: the choice slots, the option ids each slot takes, and the offers of
//! the core species, classes and backgrounds with whether they are in play.
//!
//! Owns `hero_offers` (every core species, class and background with its status), `build_groups`
//! (the groups of choices a build asks for, given the picks made so far: a species asks for its
//! own sub-choices, a class for its skills and kit, a background for its increases) and the
//! conversion between a typed value and its option id. A build is a map from slot to option id;
//! option ids are kebab-case like the rest of the creation vocabulary (`animal-handling`,
//! `high-elf`). Never does: check that the picks agree with one another (`derive` does), or offer
//! an option the SRD does not print.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::catalog::{
    BACKGROUND_GOLD_GP, BackgroundTool, ClassTools, CoreSpecies, HeroBackground, HeroClass, Kit,
    Tool, ToolKind, Weapon,
};
use crate::creation_build::{CreationOption, OptionGroup, option_label};
use crate::listed::listed_enum;
use crate::local_journey::{ALIGNMENTS, GAMING_SETS, OriginFeat, STANDARD_LANGUAGES};
use crate::roll::{Ability, Skill};
use crate::species::choice_groups;
use crate::spells::{SpellFacts, spell, wizard_spells};

pub const SPECIES: &str = "species";
pub const CLASS: &str = "class";
pub const BACKGROUND: &str = "background";
pub const ABILITY_METHOD: &str = "ability-method";
pub const STANDARD_ARRAY: &str = "standard-array";
pub const POINT_BUY: &str = "point-buy";
pub const ABILITY_INCREASE: &str = "ability-increase";
pub const TWO_AND_ONE: &str = "two-and-one";
pub const ONE_EACH: &str = "one-each";
pub const INCREASE_TWO: &str = "increase-two";
pub const INCREASE_ONE: &str = "increase-one";
pub const CLASS_SKILLS: [&str; 4] = [
    "class-skill-1",
    "class-skill-2",
    "class-skill-3",
    "class-skill-4",
];
pub const LANGUAGES: [&str; 2] = ["language-1", "language-2"];
pub const ALIGNMENT: &str = "alignment";
pub const CLASS_EQUIPMENT: &str = "class-equipment";
pub const BACKGROUND_EQUIPMENT: &str = "background-equipment";
pub const FIGHTING_STYLE: &str = "fighting-style";
pub const GAMING_SET: &str = "gaming-set";
pub const INSTRUMENTS: [&str; 3] = ["instrument-1", "instrument-2", "instrument-3"];
pub const MONK_TOOL: &str = "monk-tool";
/// The Human's Versatile Origin feat (the species group of `species::choice_groups`).
pub const VERSATILE: &str = "versatile";
/// The spell list of a Magic Initiate taken through Versatile.
pub const VERSATILE_SPELL_LIST: &str = "versatile-spell-list";
pub const SKILLED: [&str; 3] = ["skilled-1", "skilled-2", "skilled-3"];
/// The Rogue's two Expertise skills (Rogue level 1, SRD p. 61).
pub const EXPERTISE: [&str; 2] = ["expertise-1", "expertise-2"];
/// The Rogue's two Weapon Mastery weapons (Rogue level 1, SRD p. 62).
pub const WEAPON_MASTERY: [&str; 2] = ["mastery-1", "mastery-2"];
/// The language a Rogue knows beside Thieves' Cant (Thieves' Cant, SRD p. 62).
pub const ROGUE_LANGUAGE: &str = "rogue-language";
/// The Wizard's three cantrips (Wizard Features table, SRD p. 77).
pub const CANTRIPS: [&str; 3] = ["cantrip-1", "cantrip-2", "cantrip-3"];
/// The six level 1 spells a Wizard's spellbook starts with (Spellbook, SRD p. 78).
pub const SPELLBOOK: [&str; 6] = [
    "spellbook-1",
    "spellbook-2",
    "spellbook-3",
    "spellbook-4",
    "spellbook-5",
    "spellbook-6",
];
/// The four spells a level 1 Wizard prepares from the spellbook (Prepared Spells, SRD pp. 77-78).
pub const PREPARED: [&str; 4] = ["prepared-1", "prepared-2", "prepared-3", "prepared-4"];
/// The Rare Languages table (SRD p. 20), Thieves' Cant aside (a Rogue knows it already).
pub const RARE_LANGUAGES: [&str; 8] = [
    "abyssal",
    "celestial",
    "deep-speech",
    "druidic",
    "infernal",
    "primordial",
    "sylvan",
    "undercommon",
];
/// The classes whose level 1 features are in play.
pub const PLAYABLE_CLASSES: [HeroClass; 3] =
    [HeroClass::Fighter, HeroClass::Rogue, HeroClass::Wizard];
/// The two species groups that grant a skill.
pub const SPECIES_SKILL_GROUPS: [&str; 2] = ["keen-senses", "human-skill"];

/// The equipment choices of a class or background, in table order.
pub const KIT_CHOICES: [&str; 3] = ["a", "b", "c"];
/// The standard array (SRD p. 21).
pub const STANDARD_SCORES: [u8; 6] = [15, 14, 13, 12, 10, 8];
/// The lowest and highest score point buy may give, and what each costs (SRD p. 21).
pub const POINT_COSTS: [(u8, u8); 8] = [
    (8, 0),
    (9, 1),
    (10, 2),
    (11, 3),
    (12, 4),
    (13, 5),
    (14, 7),
    (15, 9),
];
/// The points point buy spends (SRD p. 21).
pub const POINT_BUY_BUDGET: u8 = 27;
/// The spell lists Magic Initiate draws from (SRD p. 87).
pub const MAGIC_INITIATE_LISTS: [&str; 3] = ["cleric", "druid", "wizard"];

listed_enum! {
    keyed
    /// The Fighting Style feats of SRD 5.2.1 (pp. 87-88).
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum FightingStyle {
        Archery,
        Defense,
        GreatWeaponFighting,
        TwoWeaponFighting,
    }
}

impl FightingStyle {
    pub fn key(self) -> &'static str {
        match self {
            Self::Archery => "archery",
            Self::Defense => "defense",
            Self::GreatWeaponFighting => "great_weapon_fighting",
            Self::TwoWeaponFighting => "two_weapon_fighting",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Archery => "Archery",
            Self::Defense => "Defense",
            Self::GreatWeaponFighting => "Great Weapon Fighting",
            Self::TwoWeaponFighting => "Two-Weapon Fighting",
        }
    }
}

listed_enum! {
    keyed
    /// Why an offer or a part of a sheet is design only ("Not in play yet").
    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum DesignOnlyReason {
        /// The source prints it, but the rules for it are not built yet.
        NotInPlayYet,
        /// The 2024 core rules have it, but its source is not admitted (SRD 5.2.1 does not print
        /// it), so it is a name and a story only.
        SourceNotAdmitted,
    }
}

impl DesignOnlyReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::NotInPlayYet => "not_in_play_yet",
            Self::SourceNotAdmitted => "source_not_admitted",
        }
    }
}

/// Whether an offer can be played.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OfferStatus {
    Playable,
    /// Designable and shown on the sheet, marked "Not in play yet", never played as another option.
    DesignOnly {
        reason: DesignOnlyReason,
        note: String,
    },
}

/// One core species, class or background on offer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HeroOffer {
    pub id: String,
    pub title: String,
    #[serde(flatten)]
    pub status: OfferStatus,
}

/// Every core species, class and background, each with its status.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HeroOffers {
    pub species: Vec<HeroOffer>,
    pub classes: Vec<HeroOffer>,
    pub backgrounds: Vec<HeroOffer>,
}

const SOURCE_NOTE: &str = "SRD 5.2.1 does not print it; it is a name and a story until its \
    source is admitted.";

fn offer(id: &str, title: &str, status: OfferStatus) -> HeroOffer {
    HeroOffer {
        id: id.to_owned(),
        title: title.to_owned(),
        status,
    }
}

fn not_admitted() -> OfferStatus {
    OfferStatus::DesignOnly {
        reason: DesignOnlyReason::SourceNotAdmitted,
        note: SOURCE_NOTE.to_owned(),
    }
}

/// Every core species, class and background with its status. A species of the SRD is playable
/// (its traits not in play yet are marked on the sheet); the Fighter's, Rogue's and Wizard's class
/// features are in play; a background the SRD does not print is design only.
pub fn hero_offers() -> HeroOffers {
    let species = CoreSpecies::ALL
        .iter()
        .map(|species| {
            let status = match species.srd() {
                Some(_) => OfferStatus::Playable,
                None => not_admitted(),
            };
            offer(&option_id(species.key()), species.title(), status)
        })
        .collect();
    let classes = HeroClass::ALL
        .iter()
        .map(|class| {
            let status = if PLAYABLE_CLASSES.contains(class) {
                OfferStatus::Playable
            } else {
                OfferStatus::DesignOnly {
                    reason: DesignOnlyReason::NotInPlayYet,
                    note: format!(
                        "The {} rolls ability checks and saving throws from its sheet; its class \
                            features ({}) are not in play yet.",
                        class.title(),
                        class.level_one_features().join(", ")
                    ),
                }
            };
            offer(&option_id(class.key()), class.title(), status)
        })
        .collect();
    let backgrounds = HeroBackground::ALL
        .iter()
        .map(|background| {
            let status = match background.srd() {
                Some(_) => OfferStatus::Playable,
                None => not_admitted(),
            };
            offer(&option_id(background.key()), background.title(), status)
        })
        .collect();
    HeroOffers {
        species,
        classes,
        backgrounds,
    }
}

/// Whether option `id` of group `group` is design only (a species, class or background offered
/// "Not in play yet"), which a roll never picks: a player may still choose it.
pub(super) fn is_design_only(offers: &HeroOffers, group: &str, id: &str) -> bool {
    let list = match group {
        SPECIES => &offers.species,
        CLASS => &offers.classes,
        BACKGROUND => &offers.backgrounds,
        _ => return false,
    };
    list.iter()
        .any(|offer| offer.id == id && offer.status != OfferStatus::Playable)
}

/// The option id of a value whose key is `key`: the key with `-` for `_`.
pub fn option_id(key: &str) -> String {
    key.replace('_', "-")
}

/// The value of `all` whose option id is `id`.
pub fn from_option_id<T: Copy>(all: &[T], key: fn(T) -> &'static str, id: &str) -> Option<T> {
    all.iter()
        .copied()
        .find(|value| option_id(key(*value)) == id)
}

pub fn skill_of(id: &str) -> Option<Skill> {
    from_option_id(Skill::ALL, Skill::key, id)
}

pub fn ability_of(id: &str) -> Option<Ability> {
    from_option_id(Ability::ALL, Ability::key, id)
}

pub fn tool_of(id: &str) -> Option<Tool> {
    from_option_id(Tool::ALL, Tool::key, id)
}

pub fn feat_of(id: &str) -> Option<OriginFeat> {
    from_option_id(OriginFeat::ALL, OriginFeat::key, id)
}

/// The picks of a build, read as typed values where they name one.
pub(super) struct Picks<'a> {
    pub(super) map: &'a BTreeMap<String, String>,
}

impl<'a> Picks<'a> {
    pub(super) fn get(&self, slot: &str) -> Option<&'a str> {
        self.map.get(slot).map(String::as_str)
    }

    pub(super) fn species(&self) -> Option<CoreSpecies> {
        from_option_id(CoreSpecies::ALL, CoreSpecies::key, self.get(SPECIES)?)
    }

    pub(super) fn class(&self) -> Option<HeroClass> {
        from_option_id(HeroClass::ALL, HeroClass::key, self.get(CLASS)?)
    }

    pub(super) fn background(&self) -> Option<HeroBackground> {
        from_option_id(
            HeroBackground::ALL,
            HeroBackground::key,
            self.get(BACKGROUND)?,
        )
    }
}

fn options_of(ids: impl IntoIterator<Item = String>) -> Vec<CreationOption> {
    ids.into_iter()
        .map(|id| CreationOption {
            label: option_label(&id),
            id,
        })
        .collect()
}

fn group(id: &str, label: &str, slots: &[&str], options: Vec<CreationOption>) -> OptionGroup {
    OptionGroup {
        id: id.to_owned(),
        label: label.to_owned(),
        slots: slots.iter().map(|slot| (*slot).to_owned()).collect(),
        options,
    }
}

fn titled<T: Copy>(
    all: &[T],
    key: fn(T) -> &'static str,
    title: fn(T) -> &'static str,
) -> Vec<CreationOption> {
    all.iter()
        .map(|value| CreationOption {
            id: option_id(key(*value)),
            label: title(*value).to_owned(),
        })
        .collect()
}

/// The equipment choices of `kits`, `a` first, each labelled with what it holds.
fn kit_options(kits: &[Kit]) -> Vec<CreationOption> {
    KIT_CHOICES
        .iter()
        .zip(kits)
        .map(|(id, kit)| CreationOption {
            id: (*id).to_owned(),
            label: format!("{}: {}", id.to_uppercase(), kit.summary()),
        })
        .collect()
}

/// One line on why a group of the build matters, for the creator's "because" (SRD pp. 19-23).
pub fn because(group: &str) -> Option<&'static str> {
    Some(match group {
        BACKGROUND => "Your background raises three abilities and gives skills, a tool and a feat.",
        ABILITY_METHOD => "The standard array or 27 points of point buy make the six scores.",
        "ability-scores" => "Your scores set every check, save and attack you make.",
        ABILITY_INCREASE => "Your background raises its abilities: +2 and +1, or +1 to all three.",
        "increases" => "Raise what you will use most.",
        "class-skills" => "Your class trains you in these skills; you add your Proficiency Bonus.",
        "languages" => "You speak Common and two more languages.",
        ALIGNMENT => "How your hero tends to act.",
        CLASS_EQUIPMENT => "What your class starts with, or its gold to buy your own.",
        BACKGROUND_EQUIPMENT => "What your background starts with, or 50 GP.",
        FIGHTING_STYLE => "A Fighter's first trick of the trade.",
        GAMING_SET | "instruments" | MONK_TOOL => "A tool you are proficient with.",
        VERSATILE => "Humans start with an Origin feat of their choice.",
        "skilled" => "Skilled trains three more skills.",
        VERSATILE_SPELL_LIST => "Magic Initiate draws on one spell list.",
        "expertise" => "Expertise doubles your Proficiency Bonus for two skills you know.",
        "weapon-mastery" => "You use the mastery property of two kinds of weapons in combat.",
        ROGUE_LANGUAGE => "You know Thieves' Cant and one more language of your choice.",
        "cantrips" => "Cantrips cost no spell slot: cast them as often as you like.",
        "spellbook" => "Your spellbook holds six level 1 spells; its rituals need no slot.",
        "prepared-spells" => "You can cast these four spellbook spells with a spell slot.",
        _ => return None,
    })
}

fn skill_options(skills: &[Skill]) -> Vec<CreationOption> {
    options_of(skills.iter().map(|skill| option_id(skill.key())))
}

fn tool_options(kinds: &[ToolKind]) -> Vec<CreationOption> {
    Tool::ALL
        .iter()
        .filter(|tool| kinds.contains(&tool.kind()))
        .map(|tool| CreationOption {
            id: option_id(tool.key()),
            label: tool.title().to_owned(),
        })
        .collect()
}

/// Every group of choices the build asks for, given the picks made so far, in the order a creator
/// meets them. A group whose owner (species, class, background, method) is not picked, or not
/// one the offers know, is not asked yet.
pub fn build_groups(picks: &BTreeMap<String, String>) -> Vec<OptionGroup> {
    let picks = Picks { map: picks };
    let mut groups = vec![
        group(
            SPECIES,
            "Species",
            &[SPECIES],
            titled(CoreSpecies::ALL, CoreSpecies::key, CoreSpecies::title),
        ),
        group(
            CLASS,
            "Class",
            &[CLASS],
            titled(HeroClass::ALL, HeroClass::key, HeroClass::title),
        ),
        group(
            BACKGROUND,
            "Background",
            &[BACKGROUND],
            titled(
                HeroBackground::ALL,
                HeroBackground::key,
                HeroBackground::title,
            ),
        ),
    ];
    species_groups(&picks, &mut groups);
    ability_groups(&picks, &mut groups);
    if let Some(class) = picks.class() {
        class_groups(class, &picks, &mut groups);
    }
    groups.push(group(
        "languages",
        "Languages (Common, and two more)",
        &LANGUAGES,
        options_of(STANDARD_LANGUAGES.iter().map(|id| (*id).to_owned())),
    ));
    groups.push(group(
        ALIGNMENT,
        "Alignment",
        &[ALIGNMENT],
        options_of(ALIGNMENTS.iter().map(|id| (*id).to_owned())),
    ));
    if let Some(background) = picks.background().and_then(HeroBackground::srd) {
        groups.push(group(
            BACKGROUND_EQUIPMENT,
            "Background equipment",
            &[BACKGROUND_EQUIPMENT],
            kit_options(&[
                background.kit,
                Kit {
                    armor: None,
                    shield: false,
                    weapons: &[],
                    gear: &[],
                    gold_gp: BACKGROUND_GOLD_GP,
                },
            ]),
        ));
        if background.tool == BackgroundTool::GamingSet {
            groups.push(group(
                GAMING_SET,
                "Gaming set",
                &[GAMING_SET],
                options_of(GAMING_SETS.iter().map(|id| (*id).to_owned())),
            ));
        }
    }
    groups
}

fn species_groups(picks: &Picks<'_>, groups: &mut Vec<OptionGroup>) {
    let Some(species) = picks.species().and_then(CoreSpecies::srd) else {
        return;
    };
    for species_group in choice_groups(species) {
        groups.push(group(
            species_group.id,
            species_group.label,
            &[species_group.id],
            options_of(species_group.options.iter().map(|id| (*id).to_owned())),
        ));
    }
    match picks.get(VERSATILE).and_then(feat_of) {
        Some(OriginFeat::Skilled) => groups.push(group(
            "skilled",
            "Skilled (three skills)",
            &SKILLED,
            skill_options(Skill::ALL),
        )),
        Some(OriginFeat::MagicInitiate) => groups.push(group(
            VERSATILE_SPELL_LIST,
            "Magic Initiate spell list",
            &[VERSATILE_SPELL_LIST],
            options_of(MAGIC_INITIATE_LISTS.iter().map(|id| (*id).to_owned())),
        )),
        _ => {}
    }
}

fn ability_groups(picks: &Picks<'_>, groups: &mut Vec<OptionGroup>) {
    groups.push(group(
        ABILITY_METHOD,
        "How the ability scores are made",
        &[ABILITY_METHOD],
        options_of([STANDARD_ARRAY.to_owned(), POINT_BUY.to_owned()]),
    ));
    let scores: Vec<u8> = match picks.get(ABILITY_METHOD) {
        Some(STANDARD_ARRAY) => STANDARD_SCORES.to_vec(),
        Some(POINT_BUY) => POINT_COSTS.iter().map(|(score, _)| *score).collect(),
        _ => Vec::new(),
    };
    if !scores.is_empty() {
        let slots: Vec<&str> = Ability::ALL.iter().map(|ability| ability.key()).collect();
        groups.push(group(
            "ability-scores",
            "Ability scores",
            &slots,
            options_of(scores.iter().map(u8::to_string)),
        ));
    }
    let Some(background) = picks.background().and_then(HeroBackground::srd) else {
        return;
    };
    groups.push(group(
        ABILITY_INCREASE,
        "Background ability increase",
        &[ABILITY_INCREASE],
        options_of([TWO_AND_ONE.to_owned(), ONE_EACH.to_owned()]),
    ));
    if picks.get(ABILITY_INCREASE) == Some(TWO_AND_ONE) {
        groups.push(group(
            "increases",
            "Increase one by 2 and another by 1",
            &[INCREASE_TWO, INCREASE_ONE],
            options_of(
                background
                    .abilities
                    .iter()
                    .map(|ability| option_id(ability.key())),
            ),
        ));
    }
}

fn class_groups(class: HeroClass, picks: &Picks<'_>, groups: &mut Vec<OptionGroup>) {
    groups.push(group(
        "class-skills",
        "Class skills",
        &CLASS_SKILLS[..class.skill_count()],
        skill_options(class.skill_options()),
    ));
    groups.push(group(
        CLASS_EQUIPMENT,
        "Class equipment",
        &[CLASS_EQUIPMENT],
        kit_options(class.kits()),
    ));
    match class {
        HeroClass::Fighter => groups.push(group(
            FIGHTING_STYLE,
            "Fighting Style",
            &[FIGHTING_STYLE],
            titled(FightingStyle::ALL, FightingStyle::key, FightingStyle::title),
        )),
        HeroClass::Rogue => rogue_groups(picks, groups),
        HeroClass::Wizard => wizard_groups(picks, groups),
        _ => {}
    }
    match class.tools() {
        ClassTools::ThreeInstruments => groups.push(group(
            "instruments",
            "Musical instruments (three)",
            &INSTRUMENTS,
            tool_options(&[ToolKind::Instrument]),
        )),
        ClassTools::ArtisanOrInstrument => groups.push(group(
            MONK_TOOL,
            "Artisan's tools or a musical instrument",
            &[MONK_TOOL],
            tool_options(&[ToolKind::Artisan, ToolKind::Instrument]),
        )),
        ClassTools::None | ClassTools::Fixed(_) => {}
    }
}

/// The Rogue's level 1 choices (SRD pp. 61-62): Expertise in two of the skills the build is
/// proficient in, Weapon Mastery of two weapons it is proficient with, and the language beside
/// Thieves' Cant (from the Standard and Rare Languages tables, p. 20).
fn rogue_groups(picks: &Picks<'_>, groups: &mut Vec<OptionGroup>) {
    let mut proficient: Vec<Skill> = Vec::new();
    for (_, granted) in super::derive::granted_skills(picks) {
        if !proficient.contains(&granted.skill) {
            proficient.push(granted.skill);
        }
    }
    groups.push(group(
        "expertise",
        "Expertise (two of your skills)",
        &EXPERTISE,
        skill_options(&proficient),
    ));
    groups.push(group(
        "weapon-mastery",
        "Weapon Mastery (two weapons)",
        &WEAPON_MASTERY,
        Weapon::ALL
            .iter()
            .filter(|weapon| rogue_proficient(**weapon))
            .map(|weapon| CreationOption {
                id: option_id(weapon.facts().id),
                label: format!("{} ({})", weapon.facts().title, weapon.mastery().title()),
            })
            .collect(),
    ));
    groups.push(group(
        ROGUE_LANGUAGE,
        "Language beside Thieves' Cant",
        &[ROGUE_LANGUAGE],
        options_of(
            STANDARD_LANGUAGES
                .iter()
                .chain(RARE_LANGUAGES.iter())
                .map(|id| (*id).to_owned()),
        ),
    ));
}

/// Whether a Rogue is proficient with `weapon`: Simple weapons, and Martial weapons with the
/// Finesse or Light property (Core Rogue Traits, SRD p. 61).
pub(super) fn rogue_proficient(weapon: Weapon) -> bool {
    !weapon.martial() || weapon.facts().finesse || weapon.light()
}

/// The weapon whose option id is `id`.
pub(super) fn weapon_of(id: &str) -> Option<Weapon> {
    Weapon::ALL
        .iter()
        .copied()
        .find(|weapon| option_id(weapon.facts().id) == id)
}

fn spell_option(spell: &SpellFacts) -> CreationOption {
    CreationOption {
        id: spell.id.to_owned(),
        label: spell.name.to_owned(),
    }
}

/// The Wizard's level 1 choices (SRD pp. 77-78): three cantrips, six level 1 spells for the
/// spellbook, and four of the spellbook's spells prepared.
fn wizard_groups(picks: &Picks<'_>, groups: &mut Vec<OptionGroup>) {
    groups.push(group(
        "cantrips",
        "Cantrips (three)",
        &CANTRIPS,
        wizard_spells(0).map(spell_option).collect(),
    ));
    groups.push(group(
        "spellbook",
        "Spellbook (six level 1 spells)",
        &SPELLBOOK,
        wizard_spells(1).map(spell_option).collect(),
    ));
    let mut in_book: Vec<CreationOption> = Vec::new();
    for spell in SPELLBOOK
        .iter()
        .filter_map(|slot| picks.get(slot))
        .filter_map(spell)
        .filter(|spell| spell.level == 1)
    {
        if !in_book.iter().any(|option| option.id == spell.id) {
            in_book.push(spell_option(spell));
        }
    }
    groups.push(group(
        "prepared-spells",
        "Prepared spells (four from your spellbook)",
        &PREPARED,
        in_book,
    ));
}

/// One line on an option of a class's choices, for the creator: a spell's level, school, tags
/// and summary; a weapon's mastery and what it does; the ability an Expertise skill adds. None
/// for an option of any other group.
pub fn option_detail(group: &str, id: &str) -> Option<String> {
    match group {
        "cantrips" | "spellbook" | "prepared-spells" => {
            let spell = spell(id)?;
            let mut parts = vec![if spell.is_cantrip() {
                "Cantrip".to_owned()
            } else {
                format!("Level {}", spell.level)
            }];
            parts.push(spell.school.title().to_owned());
            if spell.ritual {
                parts.push("ritual".to_owned());
            }
            if spell.concentration {
                parts.push("concentration".to_owned());
            }
            parts.push(spell.summary.to_owned());
            Some(parts.join(" · "))
        }
        "weapon-mastery" => {
            let weapon = weapon_of(id)?;
            Some(format!(
                "{} · {}: {}",
                weapon.facts().title,
                weapon.mastery().title(),
                weapon.mastery().effect()
            ))
        }
        "expertise" => Some(option_label(skill_of(id)?.ability().key())),
        _ => None,
    }
}
