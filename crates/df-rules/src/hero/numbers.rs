//! The numbers of a checked build: the level 1 sheet (SRD 5.2.1 pp. 19-23).
//!
//! Owns `sheet_of`: final ability scores (base, then the background's increases, p. 21),
//! modifiers (p. 21), the level 1 Proficiency Bonus (p. 23), saving throws, skills, Hit Points
//! (p. 23), Armor Class from the starting armour and shield (pp. 23, 92), Speed, Initiative,
//! Passive Perception (p. 22), weapon attacks (pp. 23, 91), healing rolls, equipment, coins and the
//! design-only marks. The Fighter's, Rogue's and Wizard's level 1 features are applied (Expertise
//! in skill totals, spellcasting numbers and spells); another class's features are marked "Not in
//! play yet", never stood in for. Never does: check a pick (`derive` did) or read a number from
//! the player.
use std::collections::{BTreeMap, BTreeSet};

use super::catalog::{
    Armor, BackgroundTool, CHOSEN_TOOL_GEAR, CONSUMABLE_GEAR, HeroClass, Kit, SHIELD_BONUS,
    UNARMORED_BASE, WeaponReach,
};
use super::choices::{DesignOnlyReason, FightingStyle, PREPARED};
use super::derive::{Build, Increase};
use super::sheet::{
    AbilityTable, DesignMark, Explained, FeatureStatus, HeroSheet, HitPoints, SheetAttack,
    SheetHealing, SheetMastery, SheetSpell, SlotCount, Spellcasting, StartingItem, StartingKit,
    short_key,
};
use crate::creation_build::option_label;
use crate::local_journey::{OriginFeat, ability_modifier};
use crate::roll::{Ability, AttackStyle, DiceExpression, DieSize, Skill, archery_bonus};
use crate::species::not_in_play;
use crate::spells::SpellFacts;

/// The Proficiency Bonus of a level 1 character (Character Advancement table, SRD p. 23).
pub const LEVEL_ONE_PROFICIENCY_BONUS: u8 = 2;
/// The rules the sheet is derived from.
pub const HERO_RULES_SOURCE: &str = "SRD 5.2.1";
/// The version of the sheet's shape.
pub const HERO_SHEET_SCHEMA: u32 = 1;
/// Passive Perception is 10 plus the Wisdom (Perception) check modifier (SRD p. 22).
const PASSIVE_BASE: i16 = 10;
/// A weapon attack scores a Critical Hit on a 20 (SRD p. 7).
const CRITICAL_FACE: u8 = 20;
/// The level a creation sheet is at.
const CREATION_LEVEL: u8 = 1;
/// The bonus of the Defense Fighting Style to Armor Class while armoured (SRD p. 88).
const DEFENSE_BONUS: i16 = 1;
/// The level 1 feature of the Barbarian and the Monk this sheet applies (`unarmored_defense`).
const UNARMORED_DEFENSE: &str = "Unarmored Defense";
/// Speed lost in armour whose Strength score the wearer lacks (SRD p. 92).
const HEAVY_ARMOR_SPEED_LOSS_FEET: u16 = 10;
/// A spell save DC is 8 + the spellcasting ability modifier + the Proficiency Bonus (SRD p. 106).
const SPELL_SAVE_BASE: i16 = 8;
/// A level 1 Wizard's spell slots: two of level 1 (Wizard Features table, SRD p. 77).
const WIZARD_LEVEL_ONE_SLOTS: [SlotCount; 1] = [SlotCount { level: 1, count: 2 }];
/// The Rogue's Sneak Attack at level 1 (Rogue Features table, SRD p. 62).
const SNEAK_ATTACK_LEVEL_ONE: &str = "1d6";

/// A class feature in play: its key, its sentence and how it is used.
struct ClassFeature {
    key: &'static str,
    text: String,
    status: FeatureStatus,
}

/// The class features in play, how each is used, and the marks of those not in play.
struct ClassFeatures {
    features: Vec<ClassFeature>,
    marks: Vec<DesignMark>,
}

impl ClassFeatures {
    fn add(&mut self, key: &'static str, text: String, status: FeatureStatus) {
        self.features.push(ClassFeature { key, text, status });
    }
}

fn mark(subject: &str, reason: DesignOnlyReason, note: String) -> DesignMark {
    DesignMark {
        subject: subject.to_owned(),
        reason,
        note,
    }
}

fn final_scores(build: &Build) -> [u8; 6] {
    let mut scores = build.base_scores;
    match build.increase {
        Increase::None => {}
        Increase::TwoAndOne { two, one } => {
            scores[two.index()] += 2;
            scores[one.index()] += 1;
        }
        Increase::OneEach(abilities) => {
            for ability in abilities {
                scores[ability.index()] += 1;
            }
        }
    }
    scores
}

fn score_note(build: &Build) -> String {
    let method = if build.method == super::choices::STANDARD_ARRAY {
        "Standard array".to_owned()
    } else {
        let spent: u8 = build
            .base_scores
            .iter()
            .filter_map(|score| {
                super::choices::POINT_COSTS
                    .iter()
                    .find(|(costed, _)| costed == score)
                    .map(|(_, cost)| *cost)
            })
            .sum();
        format!("Point buy ({spent} points)")
    };
    let base = build
        .base_scores
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join("/");
    let background = build.background.title();
    let increase = match build.increase {
        Increase::None => format!(
            "the {background} background's increases are not in play yet (its source is not \
                admitted)"
        ),
        Increase::TwoAndOne { two, one } => format!(
            "the {background} background raises {} by 2 and {} by 1",
            two.key(),
            one.key()
        ),
        Increase::OneEach(abilities) => format!(
            "the {background} background raises {}, {} and {} by 1",
            abilities[0].key(),
            abilities[1].key(),
            abilities[2].key()
        ),
    };
    format!("{method} {base}, then {increase}.")
}

/// The modifier of every ability.
fn modifiers_of(scores: [u8; 6]) -> [i8; 6] {
    scores.map(|score| i8::try_from(ability_modifier(score)).unwrap_or(i8::MAX))
}

fn feats_of(build: &Build) -> Vec<(OriginFeat, String)> {
    let mut feats = Vec::new();
    if let Some(srd) = build.srd_background {
        let list = srd
            .feat_list
            .map(|list| format!(" ({list})"))
            .unwrap_or_default();
        feats.push((
            srd.feat,
            format!(
                "{}{list} ({} background)",
                srd.feat.title(),
                build.background.title()
            ),
        ));
    }
    if let Some(feat) = build.versatile {
        let list = build
            .versatile_list
            .as_deref()
            .map(|list| format!(" ({})", option_label(list)))
            .unwrap_or_default();
        feats.push((feat, format!("{}{list} (Human Versatile)", feat.title())));
    }
    feats
}

/// The kits the build carries: the class's choice, then the background's.
fn kits_of(build: &Build) -> Vec<Kit> {
    let mut kits = Vec::new();
    if let Some(kit) = build.class.kits().get(build.class_kit) {
        kits.push(*kit);
    }
    if let (Some(srd), Some(index)) = (build.srd_background, build.background_kit) {
        // Choice A is the background's kit; choice B is its gold.
        kits.push(match index {
            0 => srd.kit,
            _ => Kit {
                armor: None,
                shield: false,
                weapons: &[],
                gear: &[],
                gold_gp: super::catalog::BACKGROUND_GOLD_GP,
            },
        });
    }
    kits
}

/// The tool a kit's "chosen tool" stands for: the Bard's first instrument, the Monk's tool, the
/// Soldier's gaming set.
fn chosen_tool_title(build: &Build, is_class_kit: bool) -> String {
    if is_class_kit {
        build
            .class_tools
            .first()
            .map_or_else(String::new, |tool| tool.title().to_owned())
    } else {
        build.gaming_set.as_deref().map_or_else(String::new, |set| {
            format!("Gaming Set ({})", option_label(set))
        })
    }
}

fn counted(title: &str, count: u16) -> String {
    if count == 1 {
        title.to_owned()
    } else {
        format!("{title} ({count})")
    }
}

/// The price in copper of one `id` from the SRD equipment catalog, when it lists one (the catalog
/// reads a kit's own ids, such as `arrows`, itself).
fn price_of(id: &str) -> Option<u64> {
    crate::srd_equipment_prices::price_cp(id).map(|price| price.copper())
}

/// The starting equipment of a checked build, item by item, the gold apart, and the gear a ready
/// pose shows (the worn armour, the shield, the class kit's first weapon).
pub(super) fn starting_kit(build: &Build) -> StartingKit {
    let kits = kits_of(build);
    let mut items: Vec<StartingItem> = Vec::new();
    let mut add = |name: String, count: u16| {
        let id = crate::srd_equipment_prices::item_id(&name);
        match items.iter_mut().find(|item| item.id == id) {
            Some(item) => item.count = item.count.saturating_add(count),
            None => items.push(StartingItem {
                price_cp: price_of(&id),
                kept: !CONSUMABLE_GEAR.contains(&name.as_str()),
                id,
                name,
                count,
            }),
        }
    };
    for (index, kit) in kits.iter().enumerate() {
        if let Some(armor) = kit.armor {
            add(armor.facts().title.to_owned(), 1);
        }
        if kit.shield {
            add("Shield".to_owned(), 1);
        }
        for (weapon, count) in kit.weapons {
            add(weapon.facts().title.to_owned(), *count);
        }
        for (gear, count) in kit.gear {
            let name = if *gear == CHOSEN_TOOL_GEAR {
                chosen_tool_title(build, index == 0)
            } else {
                (*gear).to_owned()
            };
            add(name, *count);
        }
    }
    StartingKit {
        items,
        gold_gp: kits.iter().map(|kit| kit.gold_gp).sum(),
        armor: kits
            .iter()
            .find_map(|kit| kit.armor)
            .map(|armor| armor.facts().title.to_owned()),
        shield: kits.iter().any(|kit| kit.shield),
        main_weapon: kits
            .first()
            .and_then(|kit| kit.weapons.first())
            .map(|(weapon, _)| weapon.facts().title.to_owned()),
    }
}

fn equipment_of(build: &Build, kits: &[Kit]) -> Vec<String> {
    let mut equipment = Vec::new();
    for (index, kit) in kits.iter().enumerate() {
        if let Some(armor) = kit.armor {
            let facts = armor.facts();
            equipment.push(if facts.stealth_disadvantage {
                format!(
                    "{} (Disadvantage on Dexterity (Stealth) checks)",
                    facts.title
                )
            } else {
                facts.title.to_owned()
            });
        }
        if kit.shield {
            equipment.push("Shield".to_owned());
        }
        for (weapon, count) in kit.weapons {
            equipment.push(counted(weapon.facts().title, *count));
        }
        for (gear, count) in kit.gear {
            let title = if *gear == CHOSEN_TOOL_GEAR {
                chosen_tool_title(build, index == 0)
            } else {
                (*gear).to_owned()
            };
            equipment.push(counted(&title, *count));
        }
    }
    equipment
}

/// The ability a class's Unarmored Defense adds to 10 plus Dexterity, and whether a Shield keeps
/// it (Barbarian and Monk level 1, SRD pp. 28, 49).
fn unarmored_defense(class: HeroClass) -> Option<(Ability, bool)> {
    match class {
        HeroClass::Barbarian => Some((Ability::Constitution, true)),
        HeroClass::Monk => Some((Ability::Wisdom, false)),
        _ => None,
    }
}

fn armor_class(
    kits: &[Kit],
    class: HeroClass,
    modifiers: &AbilityTable<i8>,
    style: Option<FightingStyle>,
) -> (Explained<i16>, Option<Armor>) {
    let worn = kits.iter().find_map(|kit| kit.armor);
    let shield = kits.iter().any(|kit| kit.shield);
    let dexterity = i16::from(modifiers.get(Ability::Dexterity));
    let unarmored = unarmored_defense(class)
        .filter(|(_, with_shield)| worn.is_none() && (*with_shield || !shield));
    let (mut value, mut parts) = match worn {
        Some(armor) => {
            let facts = armor.facts();
            let added = facts
                .dexterity_cap
                .map_or(dexterity, |cap| dexterity.min(i16::from(cap)));
            let part = if facts.dexterity_cap == Some(0) {
                format!("{} {}", facts.title, facts.base)
            } else {
                format!("{} {} + Dexterity {added}", facts.title, facts.base)
            };
            (i16::from(facts.base) + added, vec![part])
        }
        None => {
            let base = i16::try_from(UNARMORED_BASE).unwrap_or(i16::MAX);
            match unarmored {
                Some((ability, _)) => {
                    let added = i16::from(modifiers.get(ability));
                    (
                        base + dexterity + added,
                        vec![format!(
                            "Unarmored Defense: {base} + Dexterity {dexterity} + {} {added}",
                            option_label(ability.key())
                        )],
                    )
                }
                None => (
                    base + dexterity,
                    vec![format!("No armor: {base} + Dexterity {dexterity}")],
                ),
            }
        }
    };
    if shield {
        let bonus = i16::try_from(SHIELD_BONUS).unwrap_or(0);
        value += bonus;
        parts.push(format!("Shield +{bonus}"));
    }
    if worn.is_some() && style == Some(FightingStyle::Defense) {
        value += DEFENSE_BONUS;
        parts.push(format!("Defense +{DEFENSE_BONUS}"));
    }
    (
        Explained {
            value,
            note: format!("{}.", parts.join(", ")),
        },
        worn,
    )
}

fn dice_text(count: u8, die: DieSize, modifier: i16) -> String {
    DiceExpression {
        count,
        die,
        modifier,
    }
    .to_string()
}

fn attacks_of(
    kits: &[Kit],
    modifiers: &AbilityTable<i8>,
    style: Option<FightingStyle>,
) -> Vec<SheetAttack> {
    let mut attacks: Vec<SheetAttack> = Vec::new();
    let weapons = kits.iter().flat_map(|kit| kit.weapons.iter());
    for (weapon, _) in weapons {
        let facts = weapon.facts();
        if attacks.iter().any(|attack| attack.id == facts.id) {
            continue;
        }
        let strength = modifiers.get(Ability::Strength);
        let dexterity = modifiers.get(Ability::Dexterity);
        let ability = match facts.reach {
            WeaponReach::Ranged => Ability::Dexterity,
            _ if facts.finesse && dexterity > strength => Ability::Dexterity,
            _ => Ability::Strength,
        };
        let modifier = i16::from(modifiers.get(ability));
        let ranged = facts.reach == WeaponReach::Ranged;
        let attack_style = match style {
            Some(FightingStyle::Archery) if ranged => Some(AttackStyle::Archery),
            Some(FightingStyle::GreatWeaponFighting) if !ranged && weapon.two_handed() => {
                Some(AttackStyle::GreatWeaponFighting)
            }
            _ => None,
        };
        let style_bonus = archery_bonus(attack_style == Some(AttackStyle::Archery), ranged);
        attacks.push(SheetAttack {
            id: facts.id.to_owned(),
            name: facts.title.to_owned(),
            kind: match facts.reach {
                WeaponReach::Melee => "melee",
                WeaponReach::MeleeOrThrown => "melee_or_thrown",
                WeaponReach::Ranged => "ranged",
            },
            ability: short_key(ability),
            attack_bonus: modifier + i16::from(LEVEL_ONE_PROFICIENCY_BONUS) + style_bonus,
            damage: dice_text(facts.dice, facts.die, modifier),
            damage_type: facts.damage_type,
            style: attack_style,
        });
    }
    attacks
}

fn species_traits(build: &Build) -> (BTreeMap<String, String>, Vec<DesignMark>) {
    let mut traits = BTreeMap::new();
    let mut marks = Vec::new();
    let (Some(species), Some(options)) = (build.species.srd(), build.species_options) else {
        marks.push(mark(
            build.species.title(),
            DesignOnlyReason::SourceNotAdmitted,
            format!(
                "The {} is a 2024 core species SRD 5.2.1 does not print; its traits (size, \
                    speed and the rest) wait for the source to be admitted.",
                build.species.title()
            ),
        ));
        return (traits, marks);
    };
    let combat = options.combat_traits(species);
    if let Some(range) = combat.darkvision_ft {
        traits.insert("darkvision".to_owned(), format!("Darkvision {range} feet."));
    }
    if let Some(damage) = options.resistance(species) {
        traits.insert(
            "damage_resistance".to_owned(),
            format!("Resistance to {} damage.", damage.key()),
        );
    }
    if combat.hit_points_per_level > 0 {
        traits.insert(
            "hit_point_bonus".to_owned(),
            format!(
                "Hit Point maximum +{} per level.",
                combat.hit_points_per_level
            ),
        );
    }
    if combat.reroll_natural_ones {
        traits.insert(
            "luck".to_owned(),
            "A 1 on the d20 of a D20 Test may be rerolled; the new roll is used.".to_owned(),
        );
    }
    for gap in not_in_play(species, options) {
        marks.push(mark(
            gap,
            DesignOnlyReason::NotInPlayYet,
            format!("{}: {gap} is not in play yet.", build.species.title()),
        ));
    }
    (traits, marks)
}

fn class_features(build: &Build, modifiers: &AbilityTable<i8>) -> ClassFeatures {
    let mut features = ClassFeatures {
        features: Vec::new(),
        marks: Vec::new(),
    };
    if let Some((ability, with_shield)) = unarmored_defense(build.class) {
        let shield = if with_shield {
            "a Shield keeps it"
        } else {
            "not with a Shield"
        };
        features.add(
            "unarmored_defense",
            format!(
                "Without armor, Armor Class is 10 + Dexterity + {} ({shield}).",
                option_label(ability.key())
            ),
            FeatureStatus::InPlay,
        );
    }
    match build.class {
        HeroClass::Fighter => fighter_features(build, &mut features),
        HeroClass::Rogue => rogue_features(build, &mut features),
        HeroClass::Wizard => wizard_features(modifiers, &mut features),
        _ => {
            let title = build.class.title();
            for feature in build.class.level_one_features() {
                if *feature == UNARMORED_DEFENSE {
                    continue;
                }
                features.marks.push(mark(
                    feature,
                    DesignOnlyReason::NotInPlayYet,
                    format!("{title}: {feature} is not in play yet."),
                ));
            }
        }
    }
    features
}

fn fighter_features(build: &Build, features: &mut ClassFeatures) {
    if let Some(style) = build.style {
        let effect = match style {
            FightingStyle::Archery => "+2 to attack rolls with Ranged weapons",
            FightingStyle::Defense => "+1 Armor Class while wearing Light, Medium or Heavy armor",
            FightingStyle::GreatWeaponFighting => {
                "a 1 or 2 on a damage die of a two-handed melee attack counts as a 3"
            }
            FightingStyle::TwoWeaponFighting => {
                "the ability modifier is added to the damage of the Light weapon's extra attack"
            }
        };
        features.add(
            "fighting_style",
            format!("{} ({effect}).", style.title()),
            FeatureStatus::InPlay,
        );
    }
    features.add(
        "second_wind",
        "Bonus Action: regain 1d10 + Fighter level Hit Points; 2 uses, one regained on a Short \
            Rest and all on a Long Rest."
            .to_owned(),
        FeatureStatus::InPlay,
    );
    features.marks.push(mark(
        "Weapon Mastery",
        DesignOnlyReason::NotInPlayYet,
        "Fighter: Weapon Mastery is not in play yet (it belongs to combat).".to_owned(),
    ));
}

/// The Rogue's level 1 features (SRD pp. 61-62).
fn rogue_features(build: &Build, features: &mut ClassFeatures) {
    let expertise = build
        .expertise
        .iter()
        .map(|skill| option_label(&super::choices::option_id(skill.key())))
        .collect::<Vec<_>>()
        .join(" and ");
    features.add(
        "expertise",
        format!("{expertise}: your Proficiency Bonus is doubled for these checks."),
        FeatureStatus::InPlay,
    );
    features.add(
        "sneak_attack",
        format!(
            "Once per turn, {SNEAK_ATTACK_LEVEL_ONE} extra damage to one creature you hit with a \
                Finesse or Ranged weapon if you have Advantage, or if an ally is within 5 feet of \
                it and you don't have Disadvantage."
        ),
        FeatureStatus::AppliesInCombat,
    );
    let language = build
        .rogue_language
        .as_deref()
        .map_or_else(String::new, option_label);
    features.add(
        "thieves_cant",
        format!("You know Thieves' Cant and {language}."),
        FeatureStatus::InPlay,
    );
    let masteries = build
        .masteries
        .iter()
        .map(|weapon| format!("{} ({})", weapon.facts().title, weapon.mastery().title()))
        .collect::<Vec<_>>()
        .join(" and ");
    features.add(
        "weapon_mastery",
        format!(
            "You use the mastery properties of {masteries}; you may change the weapons after a \
                Long Rest."
        ),
        FeatureStatus::AppliesInCombat,
    );
}

/// The Wizard's level 1 features (SRD pp. 77-78).
fn wizard_features(modifiers: &AbilityTable<i8>, features: &mut ClassFeatures) {
    let (save_dc, attack_bonus) = spell_numbers(modifiers);
    features.add(
        "spellcasting",
        format!(
            "Intelligence: spell save DC {save_dc}, spell attack {attack_bonus:+}. Three \
                cantrips, four spells prepared from a six-spell spellbook, two level 1 slots \
                regained on a Long Rest."
        ),
        FeatureStatus::InPlay,
    );
    features.add(
        "ritual_adept",
        "Cast a spell with the Ritual tag from your spellbook as a Ritual, prepared or not: it \
            takes 10 minutes longer and spends no slot."
            .to_owned(),
        FeatureStatus::InPlay,
    );
    features.add(
        "arcane_recovery",
        "Once per Long Rest, when you finish a Short Rest, recover expended spell slots whose \
            levels add up to half your Wizard level, rounded up (none of level 6 or higher)."
            .to_owned(),
        FeatureStatus::InPlay,
    );
}

/// The spell save DC and spell attack bonus of an Intelligence caster at level 1 (SRD p. 106).
fn spell_numbers(modifiers: &AbilityTable<i8>) -> (i16, i16) {
    let attack =
        i16::from(modifiers.get(Ability::Intelligence)) + i16::from(LEVEL_ONE_PROFICIENCY_BONUS);
    (SPELL_SAVE_BASE + attack, attack)
}

fn sheet_spell(spell: &SpellFacts, prepared: bool) -> SheetSpell {
    SheetSpell {
        id: spell.id,
        name: spell.name,
        level: spell.level,
        school: spell.school.key(),
        ritual: spell.ritual,
        concentration: spell.concentration,
        prepared,
        casting_time: spell.casting_time,
        range: spell.range,
        components: spell.components,
        duration: spell.duration,
        summary: spell.summary,
        page: spell.page,
    }
}

/// A Wizard's spellcasting: Intelligence (p. 78), its numbers, slots and spells.
fn spellcasting_of(build: &Build, modifiers: &AbilityTable<i8>) -> Option<Spellcasting> {
    if build.class != HeroClass::Wizard {
        return None;
    }
    let (save_dc, attack_bonus) = spell_numbers(modifiers);
    let mut spells: Vec<SheetSpell> = build
        .cantrips
        .iter()
        .map(|spell| sheet_spell(spell, true))
        .collect();
    spells.extend(build.spellbook.iter().map(|spell| {
        let prepared = build.prepared.iter().any(|chosen| chosen.id == spell.id);
        sheet_spell(spell, prepared)
    }));
    Some(Spellcasting {
        ability: short_key(Ability::Intelligence),
        save_dc,
        attack_bonus,
        prepared_max: u8::try_from(PREPARED.len()).unwrap_or(u8::MAX),
        slots: WIZARD_LEVEL_ONE_SLOTS.to_vec(),
        spells,
    })
}

fn masteries_of(build: &Build) -> Vec<SheetMastery> {
    build
        .masteries
        .iter()
        .map(|weapon| SheetMastery {
            id: weapon.facts().id,
            name: weapon.facts().title,
            mastery: weapon.mastery().title(),
        })
        .collect()
}

fn feat_marks(feats: &[(OriginFeat, String)]) -> Vec<DesignMark> {
    feats
        .iter()
        .filter_map(|(feat, named)| {
            let note = match feat {
                OriginFeat::Alert => "Initiative Swap is not in play yet",
                OriginFeat::MagicInitiate => {
                    "its cantrips, level 1 spell and spellcasting ability are not in play yet"
                }
                OriginFeat::SavageAttacker => "its damage reroll is not in play yet",
                OriginFeat::Skilled => return None,
            };
            Some(mark(
                feat.title(),
                DesignOnlyReason::NotInPlayYet,
                format!("{named}: {note}."),
            ))
        })
        .collect()
}

fn background_marks(build: &Build) -> Vec<DesignMark> {
    if build.srd_background.is_some() {
        return Vec::new();
    }
    let title = build.background.title();
    vec![mark(
        title,
        DesignOnlyReason::SourceNotAdmitted,
        format!(
            "The {title} background is a 2024 core background SRD 5.2.1 does not print; its \
                ability increases, skills, tool, feat and equipment wait for the source."
        ),
    )]
}

fn tools_of(build: &Build) -> Vec<String> {
    let mut tools = Vec::new();
    match build.srd_background.map(|srd| srd.tool) {
        Some(BackgroundTool::Fixed(tool)) => tools.push(tool.title().to_owned()),
        Some(BackgroundTool::GamingSet) => tools.push(chosen_tool_title(build, false)),
        None => {}
    }
    // A proficiency is had or not: the Criminal's and the Rogue's Thieves' Tools are one.
    for tool in &build.class_tools {
        let title = tool.title().to_owned();
        if !tools.contains(&title) {
            tools.push(title);
        }
    }
    tools
}

/// The sheet of a checked build.
pub(super) fn sheet_of(id: &str, name: &str, build: &Build) -> HeroSheet {
    let scores = final_scores(build);
    let modifiers = AbilityTable::from_array(modifiers_of(scores));
    let bonus = LEVEL_ONE_PROFICIENCY_BONUS;
    let saves = build.class.saving_throws();
    let saving_throws =
        AbilityTable::from_array(Ability::ALL.iter().fold([0_i8; 6], |mut all, ability| {
            let proficient = if saves.contains(ability) { bonus } else { 0 };
            all[ability.index()] = modifiers.get(*ability).saturating_add_unsigned(proficient);
            all
        }));
    let proficient: BTreeSet<Skill> = build.skills.iter().map(|granted| granted.skill).collect();
    // Expertise doubles the Proficiency Bonus of the skill's checks (SRD p. 182).
    let skill_total = |skill: Skill| {
        let proficient = if build.expertise.contains(&skill) {
            bonus.saturating_mul(2)
        } else if proficient.contains(&skill) {
            bonus
        } else {
            0
        };
        modifiers
            .get(skill.ability())
            .saturating_add_unsigned(proficient)
    };
    let feats = feats_of(build);
    let alert = feats.iter().any(|(feat, _)| *feat == OriginFeat::Alert);
    let dexterity = modifiers.get(Ability::Dexterity);
    let constitution = modifiers.get(Ability::Constitution);
    let kits = kits_of(build);
    let (armor_class, worn) = armor_class(&kits, build.class, &modifiers, build.style);
    let (species_traits, mut not_in_play) = species_traits(build);
    let ClassFeatures {
        features: in_play,
        marks: class_marks,
    } = class_features(build, &modifiers);
    not_in_play.extend(class_marks);
    not_in_play.extend(background_marks(build));
    not_in_play.extend(feat_marks(&feats));
    let combat = build
        .species
        .srd()
        .zip(build.species_options)
        .map(|(species, options)| options.combat_traits(species));
    let species_bonus = combat.map_or(0, |traits| i32::from(traits.hit_points_per_level));
    let hit_die = build.class.hit_die();
    let maximum = (i32::from(hit_die.sides()) + i32::from(constitution) + species_bonus).max(1);
    let heavy = worn
        .and_then(|armor| armor.facts().strength_needed)
        .filter(|needed| scores[Ability::Strength.index()] < *needed);
    let speed_feet = combat.map(|traits| match heavy {
        Some(_) => traits.speed_ft.saturating_sub(HEAVY_ARMOR_SPEED_LOSS_FEET),
        None => traits.speed_ft,
    });
    let initiative = i16::from(dexterity) + if alert { i16::from(bonus) } else { 0 };
    let mut healing = Vec::new();
    if build.class == HeroClass::Fighter {
        healing.push(SheetHealing {
            id: "second_wind",
            dice: dice_text(1, DieSize::D10, i16::from(CREATION_LEVEL)),
            bonus_label: "class_level",
        });
    }
    healing.push(SheetHealing {
        id: "short_rest_hit_die",
        dice: dice_text(1, hit_die, i16::from(constitution)),
        bonus_label: "constitution",
    });
    let mut languages = vec!["Common".to_owned()];
    languages.extend(
        build
            .languages
            .iter()
            .map(|language| option_label(language)),
    );
    if let Some(language) = &build.rogue_language {
        languages.push("Thieves' Cant".to_owned());
        languages.push(option_label(language));
    }
    HeroSheet {
        schema: HERO_SHEET_SCHEMA,
        id: id.to_owned(),
        name: name.to_owned(),
        rules_source: HERO_RULES_SOURCE,
        species: build.species.title().to_owned(),
        class: build.class.title().to_owned(),
        level: CREATION_LEVEL,
        background: build.background.title().to_owned(),
        alignment: option_label(&build.alignment),
        ability_scores: AbilityTable::from_array(scores),
        ability_score_note: score_note(build),
        ability_modifiers: modifiers,
        proficiency_bonus: bonus,
        saving_throw_proficiencies: saves.iter().map(|ability| short_key(*ability)).collect(),
        saving_throws,
        skill_proficiencies: build
            .skills
            .iter()
            .map(|granted| (granted.skill.key(), granted.source.clone()))
            .collect(),
        expertise: build.expertise.iter().map(|skill| skill.key()).collect(),
        skills: Skill::ALL
            .iter()
            .map(|skill| (skill.key(), skill_total(*skill)))
            .collect(),
        tool_proficiencies: tools_of(build),
        languages,
        origin_feats: feats.iter().map(|(_, named)| named.clone()).collect(),
        species_traits,
        class_features: in_play
            .iter()
            .map(|feature| (feature.key.to_owned(), feature.text.clone()))
            .collect(),
        feature_status: in_play
            .iter()
            .map(|feature| (feature.key.to_owned(), feature.status))
            .collect(),
        weapon_masteries: masteries_of(build),
        spellcasting: spellcasting_of(build, &modifiers),
        hit_die: hit_die.key().to_owned(),
        hit_points: HitPoints {
            maximum: u32::try_from(maximum).unwrap_or(1),
            note: hit_point_note(hit_die, constitution, species_bonus),
        },
        armor_class,
        stealth_disadvantage: kits
            .iter()
            .find_map(|kit| kit.armor)
            .filter(|armor| armor.facts().stealth_disadvantage)
            .map(|armor| armor.facts().title.to_owned()),
        speed_feet,
        size: combat.map(|traits| traits.size.key()),
        darkvision_feet: combat.and_then(|traits| traits.darkvision_ft),
        initiative_modifier: Explained {
            value: initiative,
            note: if alert {
                format!("Dexterity {dexterity} + Proficiency Bonus {bonus} (Alert).")
            } else {
                format!("Dexterity {dexterity}.")
            },
        },
        passive_perception: PASSIVE_BASE + i16::from(skill_total(Skill::Perception)),
        attacks: attacks_of(&kits, &modifiers, build.style),
        healing,
        equipment: equipment_of(build, &kits),
        coins_gp: kits.iter().map(|kit| kit.gold_gp).sum(),
        critical_range: vec![CRITICAL_FACE],
        not_in_play,
        abilities: scores,
        hit_die_size: hit_die,
        proficient_skills: proficient,
    }
}

fn hit_point_note(hit_die: DieSize, constitution: i8, species_bonus: i32) -> String {
    let mut note = format!(
        "{} (the {} maximum) + {constitution} (Constitution)",
        hit_die.sides(),
        hit_die.key()
    );
    if species_bonus > 0 {
        note.push_str(&format!(" + {species_bonus} (species, per level)"));
    }
    note.push_str(" at level 1.");
    note
}
