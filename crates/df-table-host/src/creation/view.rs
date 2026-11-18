//! What a seat's phone is shown of its build: the machine's state and picks, every group the build
//! asks for (with its "because" and each option's design-only note), the problems, and the sheet as
//! the phone draws it (`df_playground::pages`).
//!
//! It maps; it decides nothing. The sheet's numbers are `df_rules::hero`'s, read from the typed
//! sheet the rules derive for the seat's picks.

use df_playground::pages::creation::{
    BuildGroup, BuildHero, BuildOption, BuildState, CreationView,
};
use df_playground::pages::wire::{
    HeroSheetView, SheetAbility, SheetAttackLine, SheetFeature, SheetMark, SheetSkill, SheetSlots,
    SheetSpell, SheetSpellcasting, SheetUses,
};
use df_rules::hero::{
    HeroSheet, HeroVitals, OfferStatus, Spellcasting, because, build_groups, hero_offers,
    option_detail,
};
use df_rules::roll::{Ability, Skill};
use df_table::creation::{Derivation, SeatCreation, SeatCreationState};

use super::rules::HeroRules;

fn state_of(state: SeatCreationState) -> BuildState {
    match state {
        SeatCreationState::Drafting => BuildState::Drafting,
        SeatCreationState::Reviewing => BuildState::Reviewing,
        SeatCreationState::Ready => BuildState::Ready,
    }
}

/// The words of a key: `animal_handling` reads "Animal Handling".
fn titled(key: &str) -> String {
    key.split('_')
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(characters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The sheet as the phone shows it.
pub fn sheet_view(sheet: &HeroSheet) -> HeroSheetView {
    let abilities = Ability::ALL
        .iter()
        .map(|ability| {
            let short = df_rules::hero::short_key(*ability);
            SheetAbility {
                key: short.to_owned(),
                roll_key: ability.key().to_owned(),
                name: titled(ability.key()),
                score: sheet.ability_scores.get(*ability),
                modifier: sheet.ability_modifiers.get(*ability),
                save: sheet.saving_throws.get(*ability),
                save_proficient: sheet.saving_throw_proficiencies.contains(&short),
            }
        })
        .collect();
    let skills = Skill::ALL
        .iter()
        .map(|skill| SheetSkill {
            key: skill.key().to_owned(),
            name: titled(skill.key()),
            ability: titled(skill.ability().key()),
            bonus: sheet.skills.get(skill.key()).copied().unwrap_or_default(),
            source: sheet.skill_proficiencies.get(skill.key()).cloned(),
            expertise: sheet.expertise.contains(&skill.key()),
            disadvantage: (*skill == Skill::Stealth)
                .then(|| sheet.stealth_disadvantage.clone())
                .flatten(),
        })
        .collect();
    let named = |entries: &std::collections::BTreeMap<String, String>| {
        entries
            .iter()
            .map(|(name, text)| SheetFeature {
                name: titled(name),
                text: text.clone(),
                status: sheet
                    .feature_status
                    .get(name)
                    .map(|status| status.key().to_owned()),
            })
            .collect::<Vec<_>>()
    };
    let mut features = named(&sheet.class_features);
    features.extend(named(&sheet.species_traits));
    HeroSheetView {
        species: sheet.species.clone(),
        class: sheet.class.clone(),
        background: sheet.background.clone(),
        alignment: sheet.alignment.clone(),
        level: sheet.level,
        hit_points: sheet.hit_points.maximum,
        armor_class: sheet.armor_class.value,
        armor_note: sheet.armor_class.note.clone(),
        speed_feet: sheet.speed_feet,
        initiative: sheet.initiative_modifier.value,
        passive_perception: sheet.passive_perception,
        proficiency_bonus: sheet.proficiency_bonus,
        abilities,
        skills,
        attacks: sheet
            .attacks
            .iter()
            .map(|attack| SheetAttackLine {
                name: attack.name.clone(),
                bonus: attack.attack_bonus,
                damage: attack.damage.clone(),
                damage_type: attack.damage_type.to_owned(),
            })
            .collect(),
        features,
        feats: sheet.origin_feats.clone(),
        languages: sheet.languages.clone(),
        tools: sheet.tool_proficiencies.clone(),
        equipment: sheet.equipment.clone(),
        coins_gp: sheet.coins_gp,
        current_hp: None,
        temp_hp: 0,
        hit_dice: None,
        uses: Vec::new(),
        spellcasting: sheet.spellcasting.as_ref().map(spellcasting_view),
        marks: sheet
            .not_in_play
            .iter()
            .map(|mark| SheetMark {
                subject: mark.subject.clone(),
                note: mark.note.clone(),
            })
            .collect(),
    }
}

/// The spellcasting as the phone shows it: every slot whole (`with_vitals` gives what is left).
fn spellcasting_view(casting: &Spellcasting) -> SheetSpellcasting {
    let ability = Ability::ALL
        .iter()
        .find(|ability| df_rules::hero::short_key(**ability) == casting.ability)
        .map_or_else(
            || casting.ability.to_owned(),
            |ability| titled(ability.key()),
        );
    SheetSpellcasting {
        ability,
        save_dc: casting.save_dc,
        attack_bonus: casting.attack_bonus,
        prepared_max: casting.prepared_max,
        spells: casting
            .spells
            .iter()
            .map(|spell| SheetSpell {
                id: spell.id.to_owned(),
                name: spell.name.to_owned(),
                level: spell.level,
                school: titled(spell.school),
                ritual: spell.ritual,
                concentration: spell.concentration,
                prepared: spell.prepared,
                casting_time: spell.casting_time.to_owned(),
                range: spell.range.to_owned(),
                duration: spell.duration.to_owned(),
                summary: spell.summary.to_owned(),
            })
            .collect(),
        slots: casting
            .slots
            .iter()
            .map(|slots| SheetSlots {
                level: slots.level,
                left: slots.count,
                maximum: slots.count,
            })
            .collect(),
    }
}

/// The sheet with the hero's live vitals: current and temporary Hit Points, Hit Point Dice,
/// feature uses and spell slots left.
pub fn with_vitals(mut view: HeroSheetView, vitals: Option<&HeroVitals>) -> HeroSheetView {
    let Some(vitals) = vitals else {
        return view;
    };
    view.current_hp = Some(vitals.current_hp);
    view.temp_hp = vitals.temp_hp;
    view.hit_dice = Some(format!(
        "{} of {} {}",
        vitals.hit_dice_left,
        vitals.hit_dice_max,
        vitals.hit_die.key()
    ));
    view.uses = vitals
        .features
        .iter()
        .map(|uses| SheetUses {
            name: titled(&uses.feature),
            left: uses.left,
            maximum: uses.maximum,
        })
        .collect();
    if let Some(casting) = view.spellcasting.as_mut() {
        casting.slots = vitals
            .spell_slots
            .iter()
            .map(|slots| SheetSlots {
                level: slots.level,
                left: slots.left,
                maximum: slots.maximum,
            })
            .collect();
    }
    view
}

/// The design-only note of each species, class and background on offer, by option id.
fn design_notes() -> Vec<(String, String)> {
    let offers = hero_offers();
    offers
        .species
        .into_iter()
        .chain(offers.classes)
        .chain(offers.backgrounds)
        .filter_map(|offer| match offer.status {
            OfferStatus::Playable => None,
            OfferStatus::DesignOnly { note, .. } => Some((offer.id, note)),
        })
        .collect()
}

/// The seat's build as its phone is shown it; an empty draft for a seat not in creation yet.
pub fn creation_view(creation: Option<&SeatCreation>) -> CreationView {
    let Some(creation) = creation else {
        return CreationView::default();
    };
    let notes = design_notes();
    let picks = creation.draft.picks().clone();
    let groups = build_groups(&picks)
        .into_iter()
        .map(|group| BuildGroup {
            because: because(&group.id).map(str::to_owned),
            options: group
                .options
                .into_iter()
                .map(|option| BuildOption {
                    not_in_play: notes
                        .iter()
                        .find(|(id, _)| {
                            *id == option.id
                                && matches!(group.id.as_str(), "species" | "class" | "background")
                        })
                        .map(|(_, note)| note.clone()),
                    detail: option_detail(&group.id, &option.id),
                    id: option.id,
                    label: option.label,
                })
                .collect(),
            id: group.id,
            label: group.label,
            slots: group.slots,
        })
        .collect();
    let issues = match &creation.derivation {
        Some(Derivation::Incomplete { issues }) => {
            issues.iter().map(|issue| issue.sentence.clone()).collect()
        }
        _ => Vec::new(),
    };
    let sheet = creation
        .hero
        .as_ref()
        .and_then(|hero| HeroRules::sheet(hero, &creation.draft).ok())
        .map(|sheet| sheet_view(&sheet));
    CreationView {
        state: state_of(creation.state),
        hero: creation.hero.as_ref().map(|hero| BuildHero {
            name: hero.name.clone(),
            species: hero.species.clone(),
            class: hero.class.clone(),
            portrait: hero.portrait.clone(),
        }),
        picks,
        groups,
        issues,
        sheet,
    }
}
