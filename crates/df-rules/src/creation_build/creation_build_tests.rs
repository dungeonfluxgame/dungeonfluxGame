use super::*;
use crate::local_journey::BackgroundBonus;

type Build = Vec<(String, String)>;

fn pairs(build: &Build) -> Vec<(&str, &str)> {
    build
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect()
}

/// A complete build for `species` that `validate_character` admits. `tested` is the one pick a test
/// varies: the skill and language pairs and the species defaults avoid its value, so no other pick
/// clashes.
fn sample(species: HeroSpecies, tested: Option<(&str, &str)>) -> Build {
    let avoid = tested.map(|(_, value)| value);
    let languages: Vec<&str> = STANDARD_LANGUAGES
        .iter()
        .copied()
        .filter(|language| Some(*language) != avoid)
        .take(2)
        .collect();
    let skills: Vec<&str> = FIGHTER_SKILLS
        .iter()
        .copied()
        .filter(|skill| Some(*skill) != avoid)
        .take(2)
        .collect();
    let mut build: Build = [
        ("species", species.key()),
        ("class", CLASS_OFFER),
        ("background", BACKGROUND_OFFER),
        ("array", "stalwart"),
        ("alignment", "neutral-good"),
        ("language-1", languages[0]),
        ("language-2", languages[1]),
        ("skill-1", skills[0]),
        ("skill-2", skills[1]),
        ("gaming-set", "dice"),
        ("equipment", EQUIPMENT_OFFER),
        ("style-masteries", STYLE_OFFER),
        ("allied-ties", TIES_OFFER),
    ]
    .iter()
    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
    .collect();
    if let Some((key, value)) = tested {
        set(&mut build, key, value);
    }
    let final_skills: Vec<String> = build
        .iter()
        .filter(|(key, _)| key == "skill-1" || key == "skill-2")
        .map(|(_, value)| value.clone())
        .collect();
    for group in choice_groups(species) {
        if build.iter().any(|(key, _)| key == group.id) {
            continue;
        }
        let value = group
            .options
            .iter()
            .copied()
            .find(|option| {
                !final_skills.iter().any(|skill| skill.as_str() == *option)
                    && !SOLDIER_SKILLS.contains(option)
            })
            .expect("every species group has an option outside the Soldier and skill picks");
        build.push((group.id.to_owned(), value.to_owned()));
    }
    build
}

fn set(build: &mut Build, key: &str, value: &str) {
    match build.iter_mut().find(|(picked, _)| picked == key) {
        Some(entry) => entry.1 = value.to_owned(),
        None => build.push((key.to_owned(), value.to_owned())),
    }
}

fn rejected(report: BuildReport) -> Vec<BuildProblem> {
    match report {
        BuildReport::Rejected { problems } => problems,
        BuildReport::Accepted(character) => panic!("expected a rejection, got {character:?}"),
    }
}

fn keys(problems: &[BuildProblem]) -> Vec<Option<&str>> {
    problems
        .iter()
        .map(|problem| problem.group.as_deref())
        .collect()
}

#[derive(Clone, Debug)]
enum Edit {
    Remove(usize),
    Set(usize, &'static str),
    Append(&'static str, &'static str),
}

impl Edit {
    fn apply(&self, build: &mut Build) {
        match *self {
            Edit::Remove(index) => {
                if index < build.len() {
                    build.remove(index);
                }
            }
            Edit::Set(index, value) => {
                if let Some(entry) = build.get_mut(index) {
                    entry.1 = value.to_owned();
                }
            }
            Edit::Append(key, value) => build.push((key.to_owned(), value.to_owned())),
        }
    }
}

const PROBES: [&str; 10] = [
    "bogus",
    "fighter-1",
    "soldier",
    "stalwart",
    "dice",
    "neutral",
    "dwarvish",
    "acrobatics",
    "human",
    "medium",
];

fn edits() -> Vec<Edit> {
    let mut edits = Vec::new();
    for index in 0..17 {
        edits.push(Edit::Remove(index));
        for probe in PROBES {
            edits.push(Edit::Set(index, probe));
        }
    }
    for key in ["zzz", "skill-3", "language-3", "human-skill", "keen-senses"] {
        edits.push(Edit::Append(key, "bogus"));
        edits.push(Edit::Append(key, "soldier"));
    }
    edits.push(Edit::Append("class", "fighter-1"));
    edits.push(Edit::Append("species", "human"));
    edits.push(Edit::Append("skill-1", "acrobatics"));
    edits
}

fn listing_text(options: &CreationOptions) -> String {
    let ids = |group: &OptionGroup| {
        group
            .options
            .iter()
            .map(|option| option.id.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut lines = Vec::new();
    for group in &options.groups {
        lines.push(format!(
            "{} [{}]: {}",
            group.id,
            group.slots.join(","),
            ids(group)
        ));
    }
    for offer in &options.species {
        for group in &offer.groups {
            lines.push(format!(
                "{} > {} [{}]: {}",
                offer.id,
                group.id,
                group.slots.join(","),
                ids(group)
            ));
        }
    }
    lines.join("\n")
}

fn check_equivalence(
    rules: &CreationRules<'_>,
    name: &str,
    build: &Build,
    tally: &mut (usize, usize),
) {
    let refs = pairs(build);
    match (
        validate_character(rules, name, &refs),
        validate_build(rules, name, &refs),
    ) {
        (Ok(character), BuildReport::Accepted(built)) => {
            assert_eq!(built, character, "accepted build differs: {build:?}");
            tally.0 += 1;
        }
        (Err(error), BuildReport::Rejected { problems }) => {
            assert!(!problems.is_empty(), "no problem for {build:?}");
            assert_eq!(problems[0].error, error, "first problem differs: {build:?}");
            for (index, problem) in problems.iter().enumerate() {
                assert!(!problem.sentence.is_empty(), "empty sentence: {build:?}");
                assert!(
                    !problems[..index].contains(problem),
                    "repeated problem: {build:?}"
                );
            }
            tally.1 += 1;
        }
        (old, new) => {
            panic!("validate_character gave {old:?} but validate_build gave {new:?}: {build:?}")
        }
    }
}

#[test]
fn listing_has_every_build_choice_once_with_unique_ids() {
    let listing = creation_options(&CreationRules::srd_521());
    let mut slots: Vec<&str> = listing
        .groups
        .iter()
        .flat_map(|group| group.slots.iter().map(String::as_str))
        .collect();
    slots.sort_unstable();
    let mut build_groups = BUILD_GROUPS.to_vec();
    build_groups.sort_unstable();
    assert_eq!(slots, build_groups);
    for group in &listing.groups {
        let mut ids: Vec<&str> = group
            .options
            .iter()
            .map(|option| option.id.as_str())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), group.options.len(), "duplicate in {}", group.id);
        assert!(group.options.iter().all(|option| !option.label.is_empty()));
    }
    let mut offered: Vec<&str> = listing
        .species
        .iter()
        .map(|offer| offer.id.as_str())
        .collect();
    offered.sort_unstable();
    assert_eq!(offered.len(), 9);
    for offer in &listing.species {
        let expected: Vec<&str> = choice_groups(
            HeroSpecies::from_key(&offer.id).expect("listed species is a known species"),
        )
        .iter()
        .map(|group| group.id)
        .collect();
        let offered: Vec<&str> = offer.groups.iter().map(|group| group.id.as_str()).collect();
        assert_eq!(offered, expected, "groups of {}", offer.id);
    }
}

#[test]
fn species_list_names_every_hero_species_and_its_label_is_plain() {
    fn position(species: HeroSpecies) -> usize {
        match species {
            HeroSpecies::Dragonborn => 0,
            HeroSpecies::Dwarf => 1,
            HeroSpecies::Elf => 2,
            HeroSpecies::Gnome => 3,
            HeroSpecies::Goliath => 4,
            HeroSpecies::Halfling => 5,
            HeroSpecies::Human => 6,
            HeroSpecies::Orc => 7,
            HeroSpecies::Tiefling => 8,
        }
    }
    for &species in HeroSpecies::ALL {
        assert_eq!(HeroSpecies::ALL[position(species)].key(), species.key());
    }
    assert_eq!(option_label("high-elf"), "High Elf");
    assert_eq!(option_label("sleight-of-hand"), "Sleight of Hand");
    assert_eq!(option_label("fighter-1"), "Fighter, level 1");
}

#[test]
fn build_slot_labels_cover_exactly_the_build_groups_in_order() {
    let keys: Vec<&str> = BUILD_SLOTS.iter().map(|(key, _)| *key).collect();
    assert_eq!(keys, BUILD_GROUPS);
}

#[test]
fn listing_ids_match_the_golden_list() {
    let listing = creation_options(&CreationRules::srd_521());
    let expected = [
        "species [species]: dragonborn dwarf elf gnome goliath halfling human orc tiefling",
        "class [class]: fighter-1",
        "background [background]: soldier",
        "array [array]: stalwart vanguard",
        "skills [skill-1,skill-2]: acrobatics animal-handling history insight persuasion \
            perception survival",
        "languages [language-1,language-2]: common-sign draconic dwarvish elvish giant gnomish \
            goblin halfling orc",
        "alignment [alignment]: lawful-good neutral-good chaotic-good lawful-neutral neutral \
            chaotic-neutral",
        "gaming_set [gaming-set]: dice dragonchess playing-cards three-dragon-ante",
        "equipment [equipment]: fighter-a-soldier-a",
        "style_masteries [style-masteries]: defense-greatsword-flail-javelin",
        "allied_ties [allied-ties]: join-order",
        "dragonborn > draconic-ancestry [draconic-ancestry]: black blue brass bronze copper gold \
            green red silver white",
        "elf > elven-lineage [elven-lineage]: drow high-elf wood-elf",
        "elf > keen-senses [keen-senses]: insight perception survival",
        "elf > species-spellcasting [species-spellcasting]: intelligence wisdom charisma",
        "gnome > gnomish-lineage [gnomish-lineage]: forest-gnome rock-gnome",
        "gnome > species-spellcasting [species-spellcasting]: intelligence wisdom charisma",
        "goliath > giant-ancestry [giant-ancestry]: cloud fire frost hill stone storm",
        "human > species-size [species-size]: medium small",
        "human > human-skill [human-skill]: acrobatics animal-handling arcana deception history \
            insight investigation medicine nature perception performance persuasion religion \
            sleight-of-hand stealth survival",
        "human > versatile [versatile]: tough",
        "tiefling > fiendish-legacy [fiendish-legacy]: abyssal chthonic infernal",
        "tiefling > species-size [species-size]: medium small",
        "tiefling > species-spellcasting [species-spellcasting]: intelligence wisdom charisma",
    ]
    .join("\n");
    assert_eq!(listing_text(&listing), expected);
}

#[test]
fn creation_options_round_trip_through_json() {
    let listing = creation_options(&CreationRules::srd_521());
    let json = serde_json::to_string(&listing).expect("listing serialises");
    let back: CreationOptions = serde_json::from_str(&json).expect("listing parses");
    assert_eq!(back, listing);
    assert!(json.contains("\"Fighter, level 1\""));
}

#[test]
fn every_listed_option_is_accepted_in_some_build() {
    let rules = CreationRules::srd_521();
    let listing = creation_options(&rules);
    for group in listing.groups.iter().filter(|group| group.id != "species") {
        for slot in &group.slots {
            for option in &group.options {
                let build = sample(
                    HeroSpecies::Dwarf,
                    Some((slot.as_str(), option.id.as_str())),
                );
                assert!(
                    matches!(
                        validate_build(&rules, "Brynn", &pairs(&build)),
                        BuildReport::Accepted(_)
                    ),
                    "{slot}={} refused: {build:?}",
                    option.id
                );
            }
        }
    }
    for &species in HeroSpecies::ALL {
        let offer = listing
            .species
            .iter()
            .find(|offer| offer.id == species.key())
            .expect("every species has an offer");
        let plain = sample(species, None);
        assert!(matches!(
            validate_build(&rules, "Brynn", &pairs(&plain)),
            BuildReport::Accepted(_)
        ));
        for group in &offer.groups {
            for option in &group.options {
                let build = sample(species, Some((group.id.as_str(), option.id.as_str())));
                assert!(
                    matches!(
                        validate_build(&rules, "Brynn", &pairs(&build)),
                        BuildReport::Accepted(_)
                    ),
                    "{} {}={} refused: {build:?}",
                    offer.id,
                    group.id,
                    option.id
                );
            }
        }
    }
}

#[test]
fn accepted_build_equals_validate_character_output() {
    let rules = CreationRules::srd_521();
    for &species in HeroSpecies::ALL {
        let build = sample(species, None);
        let refs = pairs(&build);
        let expected = validate_character(&rules, "Brynn", &refs).expect("sample is admitted");
        assert_eq!(
            validate_build(&rules, "Brynn", &refs),
            BuildReport::Accepted(expected)
        );
    }
}

#[test]
fn validate_build_accepts_exactly_what_validate_character_accepts() {
    let baseline = CreationRules::srd_521();
    let changed = CreationRules::new(
        baseline.ability_arrays(),
        BackgroundBonus {
            strength: 1,
            constitution: 2,
        },
        baseline.base_hit_points(),
        baseline.armor_class(),
        baseline.proficiency_bonus(),
        baseline.second_wind_uses_max(),
    )
    .expect("a changed background bonus inside its bounds is valid");
    let long_name = "A".repeat(33);
    let names = ["Brynn", "", "B@d", "Jean-Luc", long_name.as_str()];
    let edits = edits();
    let mut tally = (0, 0);
    for species in [
        HeroSpecies::Dwarf,
        HeroSpecies::Human,
        HeroSpecies::Elf,
        HeroSpecies::Tiefling,
    ] {
        let base = sample(species, None);
        check_equivalence(&CreationRules::srd_521(), "Brynn", &base, &mut tally);
        for name in names {
            check_equivalence(&CreationRules::srd_521(), name, &base, &mut tally);
        }
        for first in &edits {
            let mut once = base.clone();
            first.apply(&mut once);
            for name in names {
                check_equivalence(&CreationRules::srd_521(), name, &once, &mut tally);
            }
            check_equivalence(&changed, "Brynn", &once, &mut tally);
            for second in &edits {
                let mut twice = once.clone();
                second.apply(&mut twice);
                check_equivalence(&CreationRules::srd_521(), "Brynn", &twice, &mut tally);
            }
        }
    }
    assert!(tally.0 > 0, "no accepted build was exercised");
    assert!(tally.1 > 0, "no rejected build was exercised");
}

#[test]
fn invalid_creation_rules_cannot_be_built_so_no_build_is_judged_against_them() {
    let baseline = CreationRules::srd_521();
    assert_eq!(
        CreationRules::new(
            baseline.ability_arrays(),
            BackgroundBonus {
                strength: 9,
                constitution: 1,
            },
            baseline.base_hit_points(),
            baseline.armor_class(),
            baseline.proficiency_bonus(),
            baseline.second_wind_uses_max(),
        ),
        Err(JourneyRuleError::InvalidRules)
    );
}

#[test]
fn group_keys_are_distinct_snake_case_and_differ_from_slots_only_in_spelling() {
    let keys: Vec<&str> = Group::ALL.iter().map(|group| group.key()).collect();
    for key in &keys {
        assert!(
            key.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{key} is not snake_case"
        );
    }
    let mut unique = keys.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), keys.len());
    for group in Group::ALL {
        if let [slot] = group.slots() {
            assert_eq!(slot.replace('-', "_"), group.key());
        }
    }
}

#[test]
fn a_build_problem_serializes_its_error_by_key() {
    let build = sample(HeroSpecies::Dwarf, Some(("gaming-set", "chess")));
    let problems = rejected(validate_build(
        &CreationRules::srd_521(),
        "Brynn",
        &pairs(&build),
    ));
    let json = serde_json::to_value(&problems[0]).expect("a problem serializes");
    assert_eq!(json["error"], "invalid_choice");
    assert_eq!(json["group"], "gaming-set");
    assert_eq!(json["option_id"], "chess");
}

#[test]
fn three_independent_mistakes_yield_three_problems_in_check_order() {
    let rules = CreationRules::srd_521();
    let mut build = sample(HeroSpecies::Dwarf, None);
    set(&mut build, "class", "wizard-1");
    set(&mut build, "alignment", "chaotic-evil");
    set(&mut build, "skill-2", "acrobatics");
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&build)));
    assert_eq!(
        keys(&problems),
        [Some("class"), Some("alignment"), Some("skill-2")]
    );
    assert_eq!(
        problems
            .iter()
            .map(|problem| problem.error)
            .collect::<Vec<_>>(),
        [
            JourneyRuleError::InvalidChoice,
            JourneyRuleError::InvalidChoice,
            JourneyRuleError::InvalidChoice,
        ]
    );
    assert_eq!(problems[0].option_id.as_deref(), Some("wizard-1"));
    assert_eq!(problems[1].option_id.as_deref(), Some("chaotic-evil"));
    assert_eq!(problems[2].option_id.as_deref(), Some("acrobatics"));
    assert!(problems[0].sentence.contains("SRD p.47"));
}

#[test]
fn unknown_option_ids_are_named_and_refused() {
    let rules = CreationRules::srd_521();
    let mut build = sample(HeroSpecies::Dwarf, None);
    set(&mut build, "gaming-set", "chess");
    set(&mut build, "array", "dexterous");
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&build)));
    assert_eq!(keys(&problems), [Some("array"), Some("gaming-set")]);
    assert_eq!(problems[1].option_id.as_deref(), Some("chess"));
    assert!(problems[1].sentence.contains("chess"));
}

#[test]
fn empty_and_invalid_names_are_their_own_problem() {
    let rules = CreationRules::srd_521();
    let build = sample(HeroSpecies::Dwarf, None);
    for name in ["", "B@d", "1st"] {
        let problems = rejected(validate_build(&rules, name, &pairs(&build)));
        assert_eq!(problems.len(), 1, "name {name:?}");
        assert_eq!(problems[0].error, JourneyRuleError::InvalidName);
        assert_eq!(problems[0].group, None);
        assert!(problems[0].sentence.contains("32 characters"));
    }
}

#[test]
fn duplicate_group_keys_are_reported_once() {
    let rules = CreationRules::srd_521();
    let mut build = sample(HeroSpecies::Dwarf, None);
    build.push(("class".to_owned(), CLASS_OFFER.to_owned()));
    build.push(("class".to_owned(), CLASS_OFFER.to_owned()));
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&build)));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].error, JourneyRuleError::DuplicateChoice);
    assert_eq!(problems[0].group.as_deref(), Some("class"));
}

#[test]
fn duplicate_skill_pick_names_the_repeated_option() {
    let rules = CreationRules::srd_521();
    let mut build = sample(HeroSpecies::Dwarf, None);
    set(&mut build, "skill-2", "acrobatics");
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&build)));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].error, JourneyRuleError::InvalidChoice);
    assert_eq!(problems[0].group.as_deref(), Some("skill-2"));
    assert_eq!(problems[0].option_id.as_deref(), Some("acrobatics"));
}

#[test]
fn species_skill_cannot_repeat_a_class_skill_or_a_soldier_skill() {
    let rules = CreationRules::srd_521();
    let mut human = sample(HeroSpecies::Human, None);
    set(&mut human, "human-skill", "acrobatics");
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&human)));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].group.as_deref(), Some("human-skill"));
    assert_eq!(problems[0].option_id.as_deref(), Some("acrobatics"));

    set(&mut human, "human-skill", "athletics");
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&human)));
    assert_eq!(problems.len(), 1);
    assert!(problems[0].sentence.contains("SRD p.83"));
}

#[test]
fn skill_count_boundaries_two_is_admitted_one_and_three_are_refused() {
    let rules = CreationRules::srd_521();
    let two = sample(HeroSpecies::Dwarf, None);
    assert!(matches!(
        validate_build(&rules, "Brynn", &pairs(&two)),
        BuildReport::Accepted(_)
    ));

    let mut one = two.clone();
    one.retain(|(key, _)| key != "skill-2");
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&one)));
    assert_eq!(problems[0].error, JourneyRuleError::MissingChoice);
    assert_eq!(problems[0].group.as_deref(), Some("skill-2"));
    assert_eq!(problems[0].sentence, "No second skill was picked.");

    let mut three = two.clone();
    three.push(("skill-3".to_owned(), "history".to_owned()));
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&three)));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].group.as_deref(), Some("skill-3"));
    assert_eq!(problems[0].error, JourneyRuleError::InvalidChoice);
}

#[test]
fn language_count_boundaries_and_repeated_language() {
    let rules = CreationRules::srd_521();
    let two = sample(HeroSpecies::Dwarf, None);
    let mut none = two.clone();
    none.retain(|(key, _)| !key.starts_with("language-"));
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&none)));
    assert_eq!(problems[0].error, JourneyRuleError::MissingChoice);
    assert_eq!(keys(&problems), [Some("language-1"), Some("language-2")]);

    let mut repeated = two.clone();
    set(&mut repeated, "language-2", "common-sign");
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&repeated)));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].error, JourneyRuleError::InvalidChoice);
    assert_eq!(problems[0].group.as_deref(), Some("language-2"));
    assert!(problems[0].sentence.contains("different"));
}

#[test]
fn unknown_species_is_named() {
    let rules = CreationRules::srd_521();
    let mut build = sample(HeroSpecies::Dwarf, None);
    set(&mut build, "species", "wizard");
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&build)));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].group.as_deref(), Some("species"));
    assert_eq!(problems[0].option_id.as_deref(), Some("wizard"));
}

#[test]
fn first_release_offers_only_fighter_one_and_soldier() {
    let rules = CreationRules::srd_521();
    let mut build = sample(HeroSpecies::Dwarf, None);
    set(&mut build, "background", "acolyte");
    let problems = rejected(validate_build(&rules, "Brynn", &pairs(&build)));
    assert_eq!(problems.len(), 1);
    assert!(problems[0].sentence.contains("SRD p.83"));
}
