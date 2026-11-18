//! Creation over the real rules: a golden session of two heroes (one built pick by pick, one by
//! "Surprise me"), the sheet in the shape the check judge reads, a created hero rolling a check
//! through the production rules adapter, and the `creation` verbs' grammar.
//! TODO(DEV-12): a route test over a running host (open, begin from a look-studio hero, choose,
//! start, sheet out=) through the services router.
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use df_rules::roll::{Ability, DiceExpression, ModifierLabel, Skill};
use df_table::creation::{
    BuildDraft, CreationTable, CreationTransition, Derivation, HeroStart, SeatCreationState,
};
use df_table::identity::SeatId;
use df_table::machine::{CarryItem, Entry};

use df_playground::pages::creation::{BuildChoose, BuildPick, BuildState};

use super::cli::{CreationCommand, SeatStep, parse};
use super::desk::{summary_of, transition_line};
use super::rules::{HeroRules, hero_profile};
use super::view::creation_view;
use crate::adapters::{
    ActionBook, DfRulesOwner, DifficultyClass, EntropySource, RecordedDice, RulesCheckKit,
    RulesCheckReport, RulesCheckRequest, SeatRoster, SeededEntropy,
};
use crate::command::CommandError;

fn seat(number: u64) -> SeatId {
    SeatId::new(number)
}

fn hero(id: u64, name: &str, species: &str, class: &str) -> HeroStart {
    HeroStart {
        hero: id,
        name: name.to_owned(),
        species: species.to_owned(),
        class: class.to_owned(),
        portrait: None,
        public_story: Vec::new(),
    }
}

/// The picks that finish a Human Fighter Soldier begun from the look studio's species and class.
const FIGHTER_PICKS: [(&str, &str); 23] = [
    ("species-size", "medium"),
    ("human-skill", "perception"),
    ("versatile", "alert"),
    ("background", "soldier"),
    ("ability-method", "standard-array"),
    ("strength", "15"),
    ("dexterity", "14"),
    ("constitution", "13"),
    ("intelligence", "8"),
    ("wisdom", "10"),
    ("charisma", "12"),
    ("ability-increase", "two-and-one"),
    ("increase-two", "strength"),
    ("increase-one", "constitution"),
    ("class-skill-1", "survival"),
    ("class-skill-2", "acrobatics"),
    ("language-1", "dwarvish"),
    ("language-2", "giant"),
    ("alignment", "lawful-good"),
    ("class-equipment", "a"),
    ("background-equipment", "a"),
    ("fighting-style", "defense"),
    ("gaming-set", "dice"),
];

fn two_seats() -> CreationTable {
    CreationTable::enter(&Entry::carrying(vec![
        CarryItem::Seat(seat(1)),
        CarryItem::Seat(seat(2)),
    ]))
    .expect("enters")
}

fn sheet_of(table: &CreationTable, at: SeatId) -> serde_json::Value {
    match &table.seat(at).expect("in creation").derivation {
        Some(Derivation::Complete { sheet, .. }) => sheet.as_value().clone(),
        other => panic!("seat {at} has no sheet: {other:?}"),
    }
}

/// The fighter built pick by pick, as the dev CLI does it.
fn built_fighter() -> CreationTable {
    let mut table = two_seats();
    table
        .begin(
            seat(1),
            hero(11, "Mara Vell", "human", "fighter"),
            &HeroRules,
        )
        .expect("begins");
    for (slot, option) in FIGHTER_PICKS {
        table
            .choose(seat(1), slot, option, &HeroRules)
            .expect("chooses");
    }
    table
}

#[test]
fn a_golden_creation_session_derives_both_heroes_and_writes_its_lines() {
    let mut table = two_seats();
    let mut lines = Vec::new();
    let mut log = |transition: CreationTransition, slot: Option<&str>| {
        lines.push(transition_line(&transition, slot));
    };
    log(
        table
            .begin(
                seat(1),
                hero(11, "Mara Vell", "human", "fighter"),
                &HeroRules,
            )
            .expect("begins"),
        None,
    );
    log(
        table
            .begin(
                seat(2),
                hero(12, "Odo Brask", "dwarf", "wizard"),
                &HeroRules,
            )
            .expect("begins"),
        None,
    );
    for (slot, option) in &FIGHTER_PICKS[..22] {
        table
            .choose(seat(1), slot, option, &HeroRules)
            .expect("chooses");
    }
    let (slot, option) = FIGHTER_PICKS[22];
    log(
        table
            .choose(seat(1), slot, option, &HeroRules)
            .expect("chooses"),
        Some(slot),
    );
    let mut entropy = SeededEntropy::new(7);
    let mut pick = |count: usize| {
        usize::try_from(entropy.next_u64() % u64::try_from(count).unwrap_or(1)).unwrap_or(0)
    };
    log(
        table
            .surprise(seat(2), &HeroRules, &mut pick)
            .expect("surprises"),
        None,
    );
    log(table.submit(seat(1)).expect("submits"), None);
    log(table.submit(seat(2)).expect("submits"), None);
    log(table.reopen(seat(2)).expect("reopens"), None);
    log(table.submit(seat(2)).expect("submits again"), None);
    let wizard_marks = match &table.seat(seat(2)).expect("seated").derivation {
        Some(Derivation::Complete { marks, .. }) => marks.len(),
        other => panic!("the wizard is derived: {other:?}"),
    };
    assert_eq!(
        lines,
        [
            "CREATION transition seat=1 cause=begin from=drafting to=drafting issues=12 marks=0"
                .to_owned(),
            // A Wizard also asks for 3 cantrips, 6 spellbook spells and 4 prepared ones.
            "CREATION transition seat=2 cause=begin from=drafting to=drafting issues=21 marks=0"
                .to_owned(),
            "CREATION transition seat=1 cause=choose slot=gaming-set from=drafting to=reviewing \
                issues=0 marks=4"
                .to_owned(),
            format!(
                "CREATION transition seat=2 cause=surprise from=drafting to=reviewing issues=0 \
                    marks={wizard_marks}"
            ),
            "CREATION transition seat=1 cause=submit from=reviewing to=ready issues=0 marks=4"
                .to_owned(),
            format!(
                "CREATION transition seat=2 cause=submit from=reviewing to=ready issues=0 \
                    marks={wizard_marks}"
            ),
            format!(
                "CREATION transition seat=2 cause=reopen from=ready to=reviewing issues=0 \
                    marks={wizard_marks}"
            ),
            format!(
                "CREATION transition seat=2 cause=submit from=reviewing to=ready issues=0 \
                    marks={wizard_marks}"
            ),
        ]
    );
    let outcome = table.start().expect("every seat is ready");
    let summary: Vec<_> = outcome
        .heroes
        .iter()
        .map(|output| {
            let sheet = output.sheet.as_value();
            (
                output.seat,
                sheet["id"].clone(),
                sheet["species"].clone(),
                sheet["class"].clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                seat(1),
                serde_json::json!("hero-11"),
                serde_json::json!("Human"),
                serde_json::json!("Fighter")
            ),
            (
                seat(2),
                serde_json::json!("hero-12"),
                serde_json::json!("Dwarf"),
                serde_json::json!("Wizard")
            ),
        ]
    );
    let fighter = outcome.heroes[0].sheet.as_value();
    assert_eq!(fighter["hit_points"]["maximum"], 12);
    assert_eq!(fighter["armor_class"]["value"], 17);
    let wizard = outcome.heroes[1].sheet.as_value();
    assert!(
        !outcome.heroes[1]
            .marks
            .iter()
            .any(|mark| mark.subject == "Spellcasting"),
        "the wizard's spellcasting is in play"
    );
    for feature in ["spellcasting", "ritual_adept", "arcane_recovery"] {
        assert_eq!(wizard["feature_status"][feature], "in_play", "{feature}");
    }
    assert_eq!(wizard["spellcasting"]["ability"], "int");
    assert_eq!(
        wizard["spellcasting"]["spells"].as_array().map(Vec::len),
        Some(9)
    );
    assert!(wizard["class_features"]["second_wind"].is_null());
}

#[test]
fn the_derived_sheet_reads_in_the_check_judges_file_shape() {
    let sheet = sheet_of(&built_fighter(), seat(1));
    assert_eq!(sheet["id"], "hero-11");
    assert_eq!(sheet["proficiency_bonus"], 2);
    for short in ["str", "dex", "con", "int", "wis", "cha"] {
        assert!(sheet["ability_modifiers"][short].is_i64(), "{short}");
    }
    for short in sheet["saving_throw_proficiencies"]
        .as_array()
        .expect("a list")
    {
        assert!(matches!(short.as_str(), Some("str" | "con")));
    }
    for key in sheet["skill_proficiencies"]
        .as_object()
        .expect("a map")
        .keys()
    {
        assert!(Skill::from_key(key).is_some(), "{key} is a skill key");
    }
    assert!(sheet["initiative_modifier"]["value"].is_i64());
    for attack in sheet["attacks"].as_array().expect("a list") {
        assert!(matches!(
            attack["kind"].as_str(),
            Some("melee" | "melee_or_thrown" | "ranged")
        ));
        assert!(matches!(attack["ability"].as_str(), Some("str" | "dex")));
        let damage = attack["damage"].as_str().expect("damage text");
        assert!(DiceExpression::from_str(damage).is_ok(), "{damage}");
    }
    for healing in sheet["healing"].as_array().expect("a list") {
        let dice = healing["dice"].as_str().expect("dice text");
        assert!(DiceExpression::from_str(dice).is_ok(), "{dice}");
        let label = healing["bonus_label"].as_str().expect("a label");
        assert!(ModifierLabel::from_key(label).is_some(), "{label}");
    }
    assert_eq!(sheet["critical_range"], serde_json::json!([20]));
    assert!(
        sheet["equipment"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
}

#[test]
fn a_hero_made_by_the_creation_steps_rolls_a_check_through_the_rules_adapter() {
    let table = built_fighter();
    let creation = table.seat(seat(1)).expect("in creation");
    let hero = creation.hero.clone().expect("begun");
    let sheet = HeroRules::sheet(&hero, &creation.draft).expect("a legal build");
    let roster = SeatRoster::default()
        .with_hero(seat(1), hero_profile(&sheet).expect("a valid profile"))
        .expect("one hero");
    let kit = RulesCheckKit::new(
        Arc::new(DfRulesOwner::new(roster, ActionBook::default())),
        Arc::new(RecordedDice::seeded(3)),
    );
    let report = kit
        .run(&RulesCheckRequest {
            skill: Skill::Athletics,
            actor: 1,
            dc: Some(DifficultyClass::new(12).expect("a DC")),
        })
        .expect("the check resolves");
    let RulesCheckReport::Resolved {
        ability,
        facts,
        draws,
        ..
    } = report
    else {
        panic!("the check was declined: {report:?}");
    };
    assert_eq!(ability, Ability::Strength);
    assert_eq!(draws.len(), 1);
    let fact = |label: &str| {
        facts
            .iter()
            .find(|fact| fact.label == label)
            .map(|fact| fact.amount)
    };
    // Strength 17 (+3) and the Soldier's Athletics proficiency (+2).
    assert_eq!(fact("ability_bonus"), Some(3));
    assert_eq!(fact("proficiency_bonus"), Some(2));
    let natural = fact("natural_die").expect("a d20 face");
    assert_eq!(fact("total"), Some(natural + 5));
}

#[test]
fn the_build_draft_holds_the_species_and_class_the_look_studio_started() {
    let mut table = two_seats();
    table
        .begin(
            seat(2),
            hero(12, "Odo Brask", "dwarf", "wizard"),
            &HeroRules,
        )
        .expect("begins");
    let creation = table.seat(seat(2)).expect("in creation");
    assert_eq!(creation.state, SeatCreationState::Drafting);
    assert_eq!(
        creation.draft,
        BuildDraft::new(BTreeMap::from([
            ("class".to_owned(), "wizard".to_owned()),
            ("species".to_owned(), "dwarf".to_owned()),
        ]))
        .expect("a small draft")
    );
}

#[test]
fn the_creation_verbs_read_their_words() {
    let words = |line: &str| line.split(' ').map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(
        parse(&words("choose 1 class-skill-1 survival")),
        Ok(CreationCommand::Step {
            seat: 1,
            step: SeatStep::Choose(BuildChoose {
                picks: vec![BuildPick {
                    slot: "class-skill-1".to_owned(),
                    option: "survival".to_owned(),
                }],
                clear: Vec::new(),
            }),
        })
    );
    assert_eq!(
        parse(&words("sheet 2 out=artifacts/tmp/hero.sheet.json")),
        Ok(CreationCommand::Sheet {
            seat: 2,
            out: Some("artifacts/tmp/hero.sheet.json".into()),
        })
    );
    assert_eq!(
        parse(&words("surprise 3")),
        Ok(CreationCommand::Step {
            seat: 3,
            step: SeatStep::Surprise,
        })
    );
    assert!(matches!(
        parse(&words("begin one")),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        parse(&words("sheet 2 to=x")),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        parse(&words("make 1")),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn a_seat_is_shown_its_build_with_the_because_lines_and_the_sheet_in_numbers() {
    let table = built_fighter();
    let view = creation_view(table.seat(seat(1)));
    assert_eq!(view.state, BuildState::Reviewing);
    let background = view
        .groups
        .iter()
        .find(|group| group.id == "background")
        .expect("the background is asked");
    assert!(background.because.is_some());
    let farmer = background
        .options
        .iter()
        .find(|option| option.id == "farmer")
        .expect("every core background is offered");
    assert!(farmer.not_in_play.is_some());
    let sheet = view.sheet.expect("a legal build has its sheet");
    assert_eq!((sheet.hit_points, sheet.armor_class), (12, 17));
    let athletics = sheet
        .skills
        .iter()
        .find(|skill| skill.name == "Athletics")
        .expect("every skill is listed");
    assert_eq!(
        (athletics.bonus, athletics.source.as_deref()),
        (5, Some("Soldier background"))
    );
    assert!(sheet.abilities[0].save_proficient);
    assert_eq!(creation_view(None), Default::default());
}

#[test]
fn the_party_card_reads_a_built_heroes_hit_points_and_armor_class_by_character_id() {
    let table = built_fighter();
    let summary = summary_of(&table, "hero-11").expect("seat 1 built hero 11");
    assert_eq!(
        (
            summary.hero_id,
            summary.max_hp,
            summary.current_hp,
            summary.armor_class
        ),
        (11, 12, 12, 17)
    );
    assert_eq!(summary_of(&table, "hero-12"), None);
    assert_eq!(summary_of(&table, "danny"), None);
}

#[test]
fn a_submitted_build_starts_whole_and_its_vitals_are_kept_until_the_build_changes() {
    use df_rules::hero::VitalsChange;

    use super::store::HeroStates;
    use super::vitals::settle_vitals;
    let table = built_fighter();
    let mut heroes = HeroStates::new();
    assert!(settle_vitals(&mut heroes, &table, 11));
    let whole = heroes[&11].vitals.clone();
    assert_eq!((whole.current_hp, whole.max_hp), (12, 12));
    let hurt = whole
        .apply(&VitalsChange::Damage { amount: 5 })
        .expect("damage applies");
    heroes.get_mut(&11).expect("kept").vitals = hurt;
    assert!(
        !settle_vitals(&mut heroes, &table, 11),
        "the same build keeps its vitals"
    );
    assert_eq!(heroes[&11].vitals.current_hp, 7);
    assert!(
        !settle_vitals(&mut heroes, &table, 99),
        "no build, no vitals"
    );
}

#[test]
fn a_take_back_stays_empty_through_the_fill_until_the_player_picks_again() {
    let mut table = two_seats();
    let mut pick = |count: usize| count / 3;
    table
        .begin(
            seat(1),
            hero(41, "Ida Quell", "human", "wizard"),
            &HeroRules,
        )
        .expect("begins");
    table.fill(seat(1), &HeroRules, &mut pick).expect("fills");
    let rolled = table
        .seat(seat(1))
        .expect("in creation")
        .draft
        .picks()
        .clone();
    assert!(rolled.contains_key("cantrip-2"));
    // The quick hero rolls only a background in play (SRD 5.2.1 prints four).
    assert!(matches!(
        rolled.get("background").map(String::as_str),
        Some("acolyte" | "criminal" | "sage" | "soldier")
    ));
    let taken_back = table
        .clear(seat(1), "cantrip-2", &HeroRules)
        .expect("clears");
    assert_eq!(
        (taken_back.to, taken_back.issues),
        (Some(SeatCreationState::Drafting), 1)
    );
    let filled = table.fill(seat(1), &HeroRules, &mut pick).expect("fills");
    assert_eq!(
        (filled.to, filled.issues),
        (Some(SeatCreationState::Drafting), 1)
    );
    let draft = &table.seat(seat(1)).expect("in creation").draft;
    assert!(!draft.picks().contains_key("cantrip-2"));
    assert!(draft.cleared().contains("cantrip-2"));
    let picked = table
        .choose(seat(1), "cantrip-2", "fire-bolt", &HeroRules)
        .expect("chooses");
    assert_eq!(picked.to, Some(SeatCreationState::Reviewing));
    table.fill(seat(1), &HeroRules, &mut pick).expect("fills");
    let draft = &table.seat(seat(1)).expect("in creation").draft;
    assert_eq!(
        draft.picks().get("cantrip-2").map(String::as_str),
        Some("fire-bolt")
    );
    assert!(draft.cleared().is_empty());
}
