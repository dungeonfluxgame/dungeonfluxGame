//! "Surprise me" and "random unless chosen": a full legal build from the player's picks.
//!
//! Owns `surprise_build` (a fresh build) and `fill_build` (the player's picks kept, the rest
//! filled). `surprise_build` keeps the player's species, class and background when they are
//! offered ones, places the standard array where the rules suggest for the class (SRD p. 21),
//! gives the background's +2 and +1 to its two highest abilities, takes the first equipment
//! choice of the class and the background, and picks every other choice from the options the
//! build offers, never one another pick already grants. A roll picks only playable options: a
//! species, class or background "Not in play yet" is kept when the player chose it, never rolled.
//! `fill_build` never fills a slot the player cleared. Never does: draw a die or read entropy
//! itself (the caller's `pick` chooses an index), or propose a pick `derive_hero` would refuse.
use std::collections::{BTreeMap, BTreeSet};

use super::catalog::{CoreSpecies, HeroBackground, HeroClass};
use super::choices::{
    ABILITY_INCREASE, ABILITY_METHOD, BACKGROUND, BACKGROUND_EQUIPMENT, CLASS, CLASS_EQUIPMENT,
    CLASS_SKILLS, EXPERTISE, INCREASE_ONE, INCREASE_TWO, LANGUAGES, PREPARED, ROGUE_LANGUAGE,
    SKILLED, SPECIES, SPECIES_SKILL_GROUPS, STANDARD_ARRAY, TWO_AND_ONE, VERSATILE,
    VERSATILE_SPELL_LIST, build_groups, feat_of, hero_offers, is_design_only, option_id, skill_of,
};
use super::derive::derive_hero;
use crate::creation_build::BuildProblem;
use crate::local_journey::OriginFeat;
use crate::roll::Ability;

/// Chooses an index below the count it is given; an index out of range wraps.
pub type Pick<'a> = &'a mut dyn FnMut(usize) -> usize;

fn choose<'v, T>(options: &'v [T], pick: &mut dyn FnMut(usize) -> usize) -> Option<&'v T> {
    if options.is_empty() {
        return None;
    }
    options.get(pick(options.len()) % options.len())
}

/// A full build for the species and class of `picks` (chosen at random when missing or not
/// offered), keeping an offered background. Every other pick of `picks` is replaced.
pub fn surprise_build(
    picks: &BTreeMap<String, String>,
    pick: Pick<'_>,
) -> BTreeMap<String, String> {
    let offers = hero_offers();
    let playable = |group: &str, ids: Vec<String>| -> Vec<String> {
        ids.into_iter()
            .filter(|id| !is_design_only(&offers, group, id))
            .collect()
    };
    let kept = |slot: &str, ids: Vec<String>, pick: &mut dyn FnMut(usize) -> usize| match picks
        .get(slot)
    {
        Some(id) if ids.contains(id) => Some(id.clone()),
        _ => choose(&playable(slot, ids), pick).cloned(),
    };
    let species_ids = CoreSpecies::ALL
        .iter()
        .map(|species| option_id(species.key()))
        .collect();
    let class_ids = HeroClass::ALL
        .iter()
        .map(|class| option_id(class.key()))
        .collect();
    let srd_backgrounds = HeroBackground::ALL
        .iter()
        .filter(|background| background.srd().is_some())
        .map(|background| option_id(background.key()))
        .collect::<Vec<_>>();
    let background_ids = HeroBackground::ALL
        .iter()
        .map(|background| option_id(background.key()))
        .collect::<Vec<_>>();
    let mut built = BTreeMap::new();
    let mut set = |slot: &str, value: Option<String>| {
        if let Some(value) = value {
            built.insert(slot.to_owned(), value);
        }
    };
    set(SPECIES, kept(SPECIES, species_ids, pick));
    set(CLASS, kept(CLASS, class_ids, pick));
    let background = match picks.get(BACKGROUND) {
        Some(id) if background_ids.contains(id) => Some(id.clone()),
        _ => choose(&srd_backgrounds, pick).cloned(),
    };
    set(BACKGROUND, background);
    complete(&mut built, &BTreeSet::new(), pick);
    built
}

/// "Random unless chosen": the picks of `picks` the build still asks for and offers are kept, and
/// every other slot but those in `cleared` (the slots the player took back, left empty) is filled
/// as `surprise_build` fills it. A kept pick the rules refuse beside the others (a skill granted
/// twice, scores over the point-buy budget) gives way and is filled again; a build that still
/// does not stand is proposed afresh (`surprise_build`), its cleared slots left out.
pub fn fill_build(
    picks: &BTreeMap<String, String>,
    cleared: &BTreeSet<String>,
    pick: Pick<'_>,
) -> BTreeMap<String, String> {
    let mut built = offered_picks(picks);
    built.retain(|slot, _| !cleared.contains(slot));
    for _ in 0..FILL_ATTEMPTS {
        complete(&mut built, cleared, pick);
        let problems = match derive_hero(CHECK_ID, CHECK_NAME, &built) {
            Ok(_) => return built,
            Err(problems) => problems,
        };
        let problems: Vec<BuildProblem> = problems
            .into_iter()
            .filter(|problem| !left_empty(problem, cleared))
            .collect();
        if problems.is_empty() {
            return built;
        }
        for problem in problems {
            let Some(slot) = problem.group else {
                continue;
            };
            match slot.as_str() {
                SPECIES | CLASS | BACKGROUND => {}
                ABILITY_METHOD => {
                    for ability in Ability::ALL {
                        built.remove(ability.key());
                    }
                }
                _ => {
                    built.remove(&slot);
                }
            }
        }
        built = offered_picks(&built);
    }
    let mut fresh = surprise_build(picks, pick);
    fresh.retain(|slot, _| !cleared.contains(slot));
    fresh
}

/// Whether `problem` is only a slot the player cleared standing empty.
fn left_empty(problem: &BuildProblem, cleared: &BTreeSet<String>) -> bool {
    problem.error == crate::local_journey::JourneyRuleError::MissingChoice
        && problem
            .group
            .as_ref()
            .is_some_and(|slot| cleared.contains(slot))
}

/// How many times `fill_build` lets a refused pick give way before proposing afresh: each round
/// frees every slot the rules named, so two rounds settle any build the offers allow.
const FILL_ATTEMPTS: usize = 4;
/// The id and name a build is checked under: the picks are judged, not the name.
const CHECK_ID: &str = "hero";
const CHECK_NAME: &str = "Hero";

/// The picks the build asks for, each with an option it offers, read group by group as the
/// owners above them settle.
fn offered_picks(picks: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut kept = BTreeMap::new();
    loop {
        let before = kept.len();
        for group in build_groups(&kept) {
            for slot in &group.slots {
                if let Some(value) = picks.get(slot)
                    && group.options.iter().any(|option| &option.id == value)
                {
                    kept.insert(slot.clone(), value.clone());
                }
            }
        }
        if kept.len() == before {
            return kept;
        }
    }
}

/// Fills every empty slot but the `cleared` ones: the method, the scores and the increases
/// first, then each group.
fn complete(
    built: &mut BTreeMap<String, String>,
    cleared: &BTreeSet<String>,
    pick: &mut dyn FnMut(usize) -> usize,
) {
    for (slot, default) in [(ABILITY_METHOD, STANDARD_ARRAY), (CLASS_EQUIPMENT, "a")] {
        if !cleared.contains(slot) {
            built
                .entry(slot.to_owned())
                .or_insert_with(|| default.to_owned());
        }
    }
    fill_scores(built, cleared);
    fill_groups(built, cleared, pick);
}

/// When a score is missing, the class's suggested standard array (27 points, so it also stands
/// under point buy); when the background's increase is missing, +2 and +1 on its two highest
/// abilities (earlier abilities first on a tie); its equipment choice A.
fn fill_scores(built: &mut BTreeMap<String, String>, cleared: &BTreeSet<String>) {
    let mut proposed = built.clone();
    propose_scores(&mut proposed);
    for (slot, value) in proposed {
        if !built.contains_key(&slot) && !cleared.contains(&slot) {
            built.insert(slot, value);
        }
    }
}

/// `fill_scores` on a copy: every score, increase and kit choice it would set, without regard to
/// the slots the player cleared.
fn propose_scores(built: &mut BTreeMap<String, String>) {
    let class = built
        .get(CLASS)
        .and_then(|id| super::choices::from_option_id(HeroClass::ALL, HeroClass::key, id));
    let Some(class) = class else {
        return;
    };
    if Ability::ALL
        .iter()
        .any(|ability| !built.contains_key(ability.key()))
    {
        let scores = class.suggested_array();
        for ability in Ability::ALL {
            built.insert(
                ability.key().to_owned(),
                scores[ability.index()].to_string(),
            );
        }
    }
    let background = built.get(BACKGROUND).and_then(|id| {
        super::choices::from_option_id(HeroBackground::ALL, HeroBackground::key, id)
    });
    let Some(srd) = background.and_then(HeroBackground::srd) else {
        return;
    };
    let score_of = |ability: Ability| {
        built
            .get(ability.key())
            .and_then(|score| score.parse::<u8>().ok())
            .unwrap_or_default()
    };
    let mut ranked = srd.abilities;
    ranked.sort_by_key(|ability| std::cmp::Reverse(score_of(*ability)));
    built
        .entry(BACKGROUND_EQUIPMENT.to_owned())
        .or_insert_with(|| "a".to_owned());
    built
        .entry(ABILITY_INCREASE.to_owned())
        .or_insert_with(|| TWO_AND_ONE.to_owned());
    if built.get(ABILITY_INCREASE).map(String::as_str) != Some(TWO_AND_ONE) {
        return;
    }
    let two = built
        .entry(INCREASE_TWO.to_owned())
        .or_insert_with(|| option_id(ranked[0].key()))
        .clone();
    if !built.contains_key(INCREASE_ONE)
        && let Some(one) = ranked
            .iter()
            .map(|ability| option_id(ability.key()))
            .find(|id| *id != two)
    {
        built.insert(INCREASE_ONE.to_owned(), one);
    }
}

/// Every slot still empty, from the options the build offers, until no group asks for more. A
/// skill already granted, a value picked in the same group, a feat the background already gives
/// and that background's Magic Initiate list are passed over, and so is an option "Not in play
/// yet". A slot in `cleared` is left empty.
fn fill_groups(
    built: &mut BTreeMap<String, String>,
    cleared: &BTreeSet<String>,
    pick: &mut dyn FnMut(usize) -> usize,
) {
    let offers = hero_offers();
    loop {
        let groups = build_groups(built);
        let empty = groups.iter().flat_map(|group| {
            group
                .slots
                .iter()
                .filter(|slot| !built.contains_key(*slot) && !cleared.contains(*slot))
                .map(move |slot| (group, slot.clone()))
        });
        let Some((group, slot)) = empty.min_by_key(|(_, slot)| fill_rank(slot)) else {
            return;
        };
        let taken_skills = taken_skills(built);
        let background = built.get(BACKGROUND).and_then(|id| {
            super::choices::from_option_id(HeroBackground::ALL, HeroBackground::key, id)
        });
        let background_feat = background.and_then(HeroBackground::srd);
        let allowed: Vec<String> = group
            .options
            .iter()
            .map(|option| option.id.clone())
            .filter(|id| {
                let in_group = distinct_from(&group.slots, &slot)
                    .iter()
                    .any(|other| built.get(other) == Some(id));
                let skill_taken = skill_of(id).is_some_and(|skill| taken_skills.contains(&skill))
                    && is_skill_slot(&slot);
                let feat_repeated = slot == VERSATILE
                    && feat_of(id).is_some_and(|feat| {
                        feat != OriginFeat::Skilled
                            && background_feat.is_some_and(|srd| srd.feat == feat)
                    });
                let list_repeated = slot == VERSATILE_SPELL_LIST
                    && background_feat
                        .and_then(|srd| srd.feat_list)
                        .is_some_and(|list| list.to_lowercase() == *id);
                !in_group
                    && !skill_taken
                    && !feat_repeated
                    && !list_repeated
                    && !is_design_only(&offers, &group.id, id)
            })
            .collect();
        let Some(value) = choose(&allowed, pick) else {
            return;
        };
        built.insert(slot, value.clone());
    }
}

/// The slots whose picks `slot` must differ from: the other slots of its group, and every
/// language slot for a language (the Rogue's language is one more than the two).
fn distinct_from(group: &[String], slot: &str) -> Vec<String> {
    let languages = [LANGUAGES[0], LANGUAGES[1], ROGUE_LANGUAGE];
    if languages.contains(&slot) {
        return languages.iter().map(|other| (*other).to_owned()).collect();
    }
    group.to_vec()
}

/// The order empty slots are filled in: the skill picks with the fewest options first (Keen
/// Senses offers three skills, a class a handful), so a pick made earlier never leaves a later
/// one without an option; then the rest in the order the groups come; last the picks whose
/// options are other picks (Expertise from the skills, prepared spells from the spellbook).
fn fill_rank(slot: &str) -> u8 {
    if slot == SPECIES_SKILL_GROUPS[0] {
        0
    } else if CLASS_SKILLS.contains(&slot) {
        1
    } else if EXPERTISE.contains(&slot) || PREPARED.contains(&slot) {
        3
    } else {
        2
    }
}

fn is_skill_slot(slot: &str) -> bool {
    CLASS_SKILLS.contains(&slot) || SKILLED.contains(&slot) || SPECIES_SKILL_GROUPS.contains(&slot)
}

/// Every skill the build grants so far: the background's and every skill pick.
fn taken_skills(built: &BTreeMap<String, String>) -> Vec<crate::roll::Skill> {
    let mut taken: Vec<_> = built
        .get(BACKGROUND)
        .and_then(|id| super::choices::from_option_id(HeroBackground::ALL, HeroBackground::key, id))
        .and_then(HeroBackground::srd)
        .map(|srd| srd.skills.to_vec())
        .unwrap_or_default();
    taken.extend(
        built
            .iter()
            .filter(|(slot, _)| is_skill_slot(slot))
            .filter_map(|(_, id)| skill_of(id)),
    );
    taken
}
