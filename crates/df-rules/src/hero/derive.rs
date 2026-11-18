//! Checking a hero build and reading it into typed choices, the step before its sheet is derived.
//!
//! Owns `derive_hero`, the one entry: every problem of the picks (a missing or unoffered pick, a
//! pick the build does not ask for, and the rules between picks: the standard array used once,
//! the point-buy budget, two different increases, no skill or language twice, no non-repeatable
//! feat twice), or the sheet. The groups the build asks for come from `choices::build_groups`, so
//! what is offered and what is accepted cannot drift apart. Never does: read a number from the
//! player other than a base ability score, or guess a pick. Fails with the full list of
//! `BuildProblem`s, in the order a creator meets the groups.
use std::collections::BTreeMap;

use super::catalog::{
    BackgroundTool, ClassTools, CoreSpecies, HeroBackground, HeroClass, SrdBackground, Tool, Weapon,
};
use super::choices::{
    ABILITY_INCREASE, ABILITY_METHOD, ALIGNMENT, BACKGROUND_EQUIPMENT, CANTRIPS, CLASS_EQUIPMENT,
    CLASS_SKILLS, EXPERTISE, FIGHTING_STYLE, FightingStyle, GAMING_SET, INCREASE_ONE, INCREASE_TWO,
    INSTRUMENTS, KIT_CHOICES, LANGUAGES, MONK_TOOL, ONE_EACH, POINT_BUY, POINT_BUY_BUDGET,
    POINT_COSTS, PREPARED, Picks, ROGUE_LANGUAGE, SKILLED, SPECIES_SKILL_GROUPS, SPELLBOOK,
    STANDARD_ARRAY, STANDARD_SCORES, VERSATILE, VERSATILE_SPELL_LIST, WEAPON_MASTERY, ability_of,
    build_groups, feat_of, from_option_id, skill_of, tool_of, weapon_of,
};
use super::numbers::sheet_of;
use super::sheet::{HeroSheet, StartingKit};
use crate::creation_build::BuildProblem;
use crate::local_journey::{
    JourneyRuleError, MAXIMUM_CHARACTER_NAME_CHARACTERS, OriginFeat, valid_character_name,
};
use crate::roll::{Ability, Skill};
use crate::species::SpeciesOptions;
use crate::spells::{SpellFacts, spell};

/// How the background raises the ability scores (SRD p. 21).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Increase {
    /// No increase: the background's source is not admitted.
    None,
    TwoAndOne {
        two: Ability,
        one: Ability,
    },
    OneEach([Ability; 3]),
}

/// A skill proficiency and where it comes from, in the words of the sheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct GrantedSkill {
    pub(super) skill: Skill,
    pub(super) source: String,
}

/// A checked build as typed values.
#[derive(Clone, Debug)]
pub(super) struct Build {
    pub(super) species: CoreSpecies,
    /// The SRD species' options; `None` for a species with no admitted source.
    pub(super) species_options: Option<SpeciesOptions>,
    pub(super) class: HeroClass,
    pub(super) background: HeroBackground,
    pub(super) srd_background: Option<SrdBackground>,
    pub(super) base_scores: [u8; 6],
    pub(super) method: &'static str,
    pub(super) increase: Increase,
    /// Every skill proficiency, background first, in the order granted.
    pub(super) skills: Vec<GrantedSkill>,
    pub(super) versatile: Option<OriginFeat>,
    pub(super) versatile_list: Option<String>,
    pub(super) languages: [String; 2],
    pub(super) alignment: String,
    pub(super) class_kit: usize,
    /// The background's equipment choice; `None` when the background has no admitted source.
    pub(super) background_kit: Option<usize>,
    pub(super) style: Option<FightingStyle>,
    pub(super) gaming_set: Option<String>,
    pub(super) class_tools: Vec<Tool>,
    /// The Rogue's Expertise skills; empty for another class.
    pub(super) expertise: Vec<Skill>,
    /// The Rogue's Weapon Mastery weapons; empty for another class.
    pub(super) masteries: Vec<Weapon>,
    /// The Rogue's language beside Thieves' Cant.
    pub(super) rogue_language: Option<String>,
    /// The Wizard's cantrips, spellbook and prepared spells; empty for another class.
    pub(super) cantrips: Vec<&'static SpellFacts>,
    pub(super) spellbook: Vec<&'static SpellFacts>,
    pub(super) prepared: Vec<&'static SpellFacts>,
}

/// The starting equipment of the build of `picks`, or every problem of the picks.
pub fn starting_kit(picks: &BTreeMap<String, String>) -> Result<StartingKit, Vec<BuildProblem>> {
    let problems = check_picks(CHECK_NAME, picks);
    if !problems.is_empty() {
        return Err(problems);
    }
    read_build(picks)
        .map(|build| super::numbers::starting_kit(&build))
        .ok_or_else(|| {
            vec![problem(
                JourneyRuleError::InvalidChoice,
                None,
                None,
                "The build could not be read, though every pick is offered.".to_owned(),
            )]
        })
}

/// The name a build's equipment is checked under: the picks are judged, not the name.
const CHECK_NAME: &str = "Hero";

/// The sheet of hero `id` named `name` built from `picks`, or every problem of the picks.
pub fn derive_hero(
    id: &str,
    name: &str,
    picks: &BTreeMap<String, String>,
) -> Result<HeroSheet, Vec<BuildProblem>> {
    let problems = check_picks(name, picks);
    if !problems.is_empty() {
        return Err(problems);
    }
    let Some(build) = read_build(picks) else {
        return Err(vec![problem(
            JourneyRuleError::InvalidChoice,
            None,
            None,
            "The build could not be read, though every pick is offered.".to_owned(),
        )]);
    };
    Ok(sheet_of(id, name, &build))
}

fn problem(
    error: JourneyRuleError,
    group: Option<&str>,
    option_id: Option<&str>,
    sentence: String,
) -> BuildProblem {
    BuildProblem {
        error,
        group: group.map(str::to_owned),
        option_id: option_id.map(str::to_owned),
        sentence,
    }
}

/// Every problem of `picks`, in the order a creator meets the groups.
fn check_picks(name: &str, picks: &BTreeMap<String, String>) -> Vec<BuildProblem> {
    let mut problems = Vec::new();
    if !valid_character_name(name) {
        problems.push(problem(
            JourneyRuleError::InvalidName,
            None,
            None,
            format!(
                "The hero name must be 1 to {MAXIMUM_CHARACTER_NAME_CHARACTERS} characters: \
                    letters, joined by single spaces, apostrophes, hyphens, periods or \
                    underscores, starting with a letter and ending with a letter or period."
            ),
        ));
    }
    let groups = build_groups(picks);
    for (slot, value) in picks {
        let asked = groups
            .iter()
            .any(|group| group.slots.iter().any(|asked| asked == slot));
        if !asked {
            problems.push(problem(
                JourneyRuleError::InvalidChoice,
                Some(slot),
                Some(value),
                format!("\"{slot}\" is not a choice this build asks for."),
            ));
        }
    }
    for group in &groups {
        for slot in &group.slots {
            match picks.get(slot) {
                None => problems.push(problem(
                    JourneyRuleError::MissingChoice,
                    Some(slot),
                    None,
                    format!("Nothing is picked for {} ({slot}).", group.label),
                )),
                Some(value) if !group.options.iter().any(|option| &option.id == value) => problems
                    .push(problem(
                        JourneyRuleError::InvalidChoice,
                        Some(slot),
                        Some(value),
                        format!("\"{value}\" is not an option for {}.", group.label),
                    )),
                Some(_) => {}
            }
        }
    }
    let picks = Picks { map: picks };
    check_scores(&picks, &mut problems);
    check_increases(&picks, &mut problems);
    check_skills(&picks, &mut problems);
    check_distinct(&picks, &LANGUAGES, "language", &mut problems);
    check_distinct(&picks, &INSTRUMENTS, "instrument", &mut problems);
    check_distinct(&picks, &EXPERTISE, "Expertise skill", &mut problems);
    check_distinct(
        &picks,
        &WEAPON_MASTERY,
        "Weapon Mastery weapon",
        &mut problems,
    );
    check_distinct(&picks, &CANTRIPS, "cantrip", &mut problems);
    check_distinct(&picks, &SPELLBOOK, "spellbook spell", &mut problems);
    check_distinct(&picks, &PREPARED, "prepared spell", &mut problems);
    check_distinct(
        &picks,
        &[LANGUAGES[0], LANGUAGES[1], ROGUE_LANGUAGE],
        "language",
        &mut problems,
    );
    check_feats(&picks, &mut problems);
    problems
}

/// The base score picked for each ability, when all six are picked and are numbers.
fn base_scores(picks: &Picks<'_>) -> Option<[u8; 6]> {
    let mut scores = [0_u8; 6];
    for ability in Ability::ALL {
        scores[ability.index()] = picks.get(ability.key())?.parse().ok()?;
    }
    Some(scores)
}

fn point_cost(score: u8) -> Option<u8> {
    POINT_COSTS
        .iter()
        .find(|(costed, _)| *costed == score)
        .map(|(_, cost)| *cost)
}

fn check_scores(picks: &Picks<'_>, problems: &mut Vec<BuildProblem>) {
    let Some(scores) = base_scores(picks) else {
        return;
    };
    match picks.get(ABILITY_METHOD) {
        Some(STANDARD_ARRAY) => {
            let mut sorted = scores;
            sorted.sort_unstable();
            let mut standard = STANDARD_SCORES;
            standard.sort_unstable();
            if sorted != standard {
                problems.push(problem(
                    JourneyRuleError::InvalidChoice,
                    Some(ABILITY_METHOD),
                    Some(STANDARD_ARRAY),
                    "The standard array gives each of 15, 14, 13, 12, 10 and 8 to exactly one \
                        ability (SRD p. 21)."
                        .to_owned(),
                ));
            }
        }
        Some(POINT_BUY) => {
            let Some(spent) = scores
                .iter()
                .map(|score| point_cost(*score).map(u16::from))
                .sum::<Option<u16>>()
            else {
                return;
            };
            if spent > u16::from(POINT_BUY_BUDGET) {
                problems.push(problem(
                    JourneyRuleError::InvalidChoice,
                    Some(ABILITY_METHOD),
                    Some(POINT_BUY),
                    format!(
                        "These scores cost {spent} points; point buy has {POINT_BUY_BUDGET} to \
                            spend (SRD p. 21)."
                    ),
                ));
            }
        }
        _ => {}
    }
}

fn check_increases(picks: &Picks<'_>, problems: &mut Vec<BuildProblem>) {
    if let (Some(two), Some(one)) = (picks.get(INCREASE_TWO), picks.get(INCREASE_ONE))
        && two == one
    {
        problems.push(problem(
            JourneyRuleError::InvalidChoice,
            Some(INCREASE_ONE),
            Some(one),
            "The +1 goes to a different ability than the +2 (SRD p. 83).".to_owned(),
        ));
    }
}

/// The species' skill slot and the trait that grants it.
fn species_skill_source(slot: &str) -> &'static str {
    if slot == SPECIES_SKILL_GROUPS[0] {
        "Elf Keen Senses"
    } else {
        "Human Skillful"
    }
}

/// Every skill proficiency the picks grant, in order (background, class, species, Skilled), each
/// with its slot; a skill that does not read is left out (its pick has its own problem).
pub(super) fn granted_skills(picks: &Picks<'_>) -> Vec<(Option<&'static str>, GrantedSkill)> {
    let mut granted = Vec::new();
    if let Some(background) = picks.background()
        && let Some(srd) = background.srd()
    {
        for skill in srd.skills {
            let source = format!("{} background", background.title());
            granted.push((None, GrantedSkill { skill, source }));
        }
    }
    let class_title = picks.class().map_or("Class", HeroClass::title);
    let mut chosen: Vec<(&'static str, String)> = CLASS_SKILLS
        .iter()
        .map(|slot| (*slot, format!("{class_title} class choice")))
        .collect();
    chosen.extend(
        SPECIES_SKILL_GROUPS
            .iter()
            .map(|slot| (*slot, species_skill_source(slot).to_owned())),
    );
    chosen.extend(
        SKILLED
            .iter()
            .map(|slot| (*slot, "Skilled feat".to_owned())),
    );
    for (slot, source) in chosen {
        if let Some(skill) = picks.get(slot).and_then(skill_of) {
            granted.push((Some(slot), GrantedSkill { skill, source }));
        }
    }
    granted
}

fn check_skills(picks: &Picks<'_>, problems: &mut Vec<BuildProblem>) {
    let granted = granted_skills(picks);
    for (index, (slot, later)) in granted.iter().enumerate() {
        let Some(slot) = slot else {
            continue;
        };
        if let Some((_, earlier)) = granted[..index]
            .iter()
            .find(|(_, earlier)| earlier.skill == later.skill)
        {
            problems.push(problem(
                JourneyRuleError::InvalidChoice,
                Some(slot),
                picks.get(slot),
                format!(
                    "{} is already a proficiency from the {}; pick a different skill.",
                    later.skill.key(),
                    earlier.source
                ),
            ));
        }
    }
}

fn check_distinct(
    picks: &Picks<'_>,
    slots: &[&'static str],
    noun: &str,
    problems: &mut Vec<BuildProblem>,
) {
    for (index, slot) in slots.iter().enumerate() {
        let Some(value) = picks.get(slot) else {
            continue;
        };
        if slots[..index]
            .iter()
            .any(|earlier| picks.get(earlier) == Some(value))
        {
            problems.push(problem(
                JourneyRuleError::InvalidChoice,
                Some(slot),
                Some(value),
                format!("The {noun} \"{value}\" is picked twice; pick a different one."),
            ));
        }
    }
}

/// A feat is taken twice only when it is repeatable (SRD p. 87): Skilled, and Magic Initiate with
/// a different spell list each time.
fn check_feats(picks: &Picks<'_>, problems: &mut Vec<BuildProblem>) {
    let Some(versatile) = picks.get(VERSATILE).and_then(feat_of) else {
        return;
    };
    let Some(background) = picks.background().and_then(HeroBackground::srd) else {
        return;
    };
    if versatile != background.feat {
        return;
    }
    match versatile {
        OriginFeat::Skilled => {}
        OriginFeat::MagicInitiate => {
            let list = picks.get(VERSATILE_SPELL_LIST);
            if list.is_some()
                && list.map(str::to_owned) == background.feat_list.map(str::to_lowercase)
            {
                problems.push(problem(
                    JourneyRuleError::InvalidChoice,
                    Some(VERSATILE_SPELL_LIST),
                    list,
                    "Magic Initiate taken twice needs a different spell list each time (SRD p. 87)."
                        .to_owned(),
                ));
            }
        }
        OriginFeat::Alert | OriginFeat::SavageAttacker => problems.push(problem(
            JourneyRuleError::InvalidChoice,
            Some(VERSATILE),
            picks.get(VERSATILE),
            format!(
                "{} is already the background's feat and is not repeatable (SRD p. 87).",
                versatile.title()
            ),
        )),
    }
}

/// The checked picks as typed values; `None` only if a pick that passed the checks does not read.
fn read_build(map: &BTreeMap<String, String>) -> Option<Build> {
    let picks = Picks { map };
    let species = picks.species()?;
    let class = picks.class()?;
    let background = picks.background()?;
    let srd_background = background.srd();
    let species_options = match species.srd() {
        Some(srd) => Some(SpeciesOptions::from_group_picks(srd, |slot| picks.get(slot)).ok()?),
        None => None,
    };
    let method = match picks.get(ABILITY_METHOD)? {
        STANDARD_ARRAY => STANDARD_ARRAY,
        _ => POINT_BUY,
    };
    let increase = match srd_background {
        None => Increase::None,
        Some(srd) if picks.get(ABILITY_INCREASE)? == ONE_EACH => Increase::OneEach(srd.abilities),
        Some(_) => Increase::TwoAndOne {
            two: ability_of(picks.get(INCREASE_TWO)?)?,
            one: ability_of(picks.get(INCREASE_ONE)?)?,
        },
    };
    let kit_index = |slot: &str| {
        KIT_CHOICES
            .iter()
            .position(|id| Some(*id) == picks.get(slot))
    };
    let class_tools = match class.tools() {
        ClassTools::None => Vec::new(),
        ClassTools::Fixed(tool) => vec![tool],
        ClassTools::ThreeInstruments => INSTRUMENTS
            .iter()
            .map(|slot| picks.get(slot).and_then(tool_of))
            .collect::<Option<Vec<_>>>()?,
        ClassTools::ArtisanOrInstrument => vec![tool_of(picks.get(MONK_TOOL)?)?],
    };
    let rogue = class == HeroClass::Rogue;
    let wizard = class == HeroClass::Wizard;
    let spells = |slots: &[&str]| read_slots(&picks, wizard, slots, spell);
    Some(Build {
        species,
        species_options,
        class,
        background,
        srd_background,
        base_scores: base_scores(&picks)?,
        method,
        increase,
        skills: granted_skills(&picks)
            .into_iter()
            .map(|(_, granted)| granted)
            .collect(),
        versatile: picks.get(VERSATILE).and_then(feat_of),
        versatile_list: picks.get(VERSATILE_SPELL_LIST).map(str::to_owned),
        languages: [
            picks.get(LANGUAGES[0])?.to_owned(),
            picks.get(LANGUAGES[1])?.to_owned(),
        ],
        alignment: picks.get(ALIGNMENT)?.to_owned(),
        class_kit: kit_index(CLASS_EQUIPMENT)?,
        background_kit: match srd_background {
            Some(_) => Some(kit_index(BACKGROUND_EQUIPMENT)?),
            None => None,
        },
        style: match picks.get(FIGHTING_STYLE) {
            Some(id) => Some(from_option_id(FightingStyle::ALL, FightingStyle::key, id)?),
            None => None,
        },
        gaming_set: match srd_background.map(|srd| srd.tool) {
            Some(BackgroundTool::GamingSet) => Some(picks.get(GAMING_SET)?.to_owned()),
            _ => None,
        },
        class_tools,
        expertise: read_slots(&picks, rogue, &EXPERTISE, skill_of)?,
        masteries: read_slots(&picks, rogue, &WEAPON_MASTERY, weapon_of)?,
        rogue_language: match rogue {
            true => Some(picks.get(ROGUE_LANGUAGE)?.to_owned()),
            false => None,
        },
        cantrips: spells(&CANTRIPS)?,
        spellbook: spells(&SPELLBOOK)?,
        prepared: spells(&PREPARED)?,
    })
}

/// The values picked in `slots`, each read by `read`, when `asked`; empty when not asked; `None`
/// when a slot asked for is missing or does not read.
fn read_slots<T>(
    picks: &Picks<'_>,
    asked: bool,
    slots: &[&str],
    read: impl Fn(&str) -> Option<T>,
) -> Option<Vec<T>> {
    if !asked {
        return Some(Vec::new());
    }
    slots
        .iter()
        .map(|slot| picks.get(slot).and_then(&read))
        .collect()
}
