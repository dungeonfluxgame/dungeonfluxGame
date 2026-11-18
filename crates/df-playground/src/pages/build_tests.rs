//! The hero build screens and the Hero and Roll pages: what each shows and what each tap sends.
use std::collections::BTreeMap;

use super::build::{BuildInput, BuildPart, BuildTap, BuildUi, build_panels};
use super::creation::{BuildGroup, BuildHero, BuildOption, BuildState, CreationView};
use super::sheet::{hero_page, roll_page};
use super::wire::{HeroSheetView, SeatPages, SheetAbility, SheetMark, SheetSkill};
use crate::phone_kit::page::{Block, Chip};

fn option(id: &str, label: &str) -> BuildOption {
    BuildOption {
        id: id.to_owned(),
        label: label.to_owned(),
        not_in_play: None,
        detail: None,
    }
}

fn group(id: &str, slots: &[&str], options: Vec<BuildOption>) -> BuildGroup {
    BuildGroup {
        id: id.to_owned(),
        label: id.to_owned(),
        because: Some("Because.".to_owned()),
        slots: slots.iter().map(|slot| (*slot).to_owned()).collect(),
        options,
    }
}

fn sheet() -> HeroSheetView {
    HeroSheetView {
        species: "Human".to_owned(),
        class: "Fighter".to_owned(),
        background: "Soldier".to_owned(),
        alignment: "Lawful Good".to_owned(),
        level: 1,
        hit_points: 12,
        armor_class: 17,
        armor_note: "Chain Mail 16, Defense +1.".to_owned(),
        speed_feet: Some(30),
        initiative: 4,
        passive_perception: 12,
        proficiency_bonus: 2,
        abilities: vec![SheetAbility {
            key: "str".to_owned(),
            roll_key: "strength".to_owned(),
            name: "Strength".to_owned(),
            score: 17,
            modifier: 3,
            save: 5,
            save_proficient: true,
        }],
        skills: vec![
            SheetSkill {
                key: "athletics".to_owned(),
                name: "Athletics".to_owned(),
                ability: "Strength".to_owned(),
                bonus: 5,
                source: Some("Soldier background".to_owned()),
                disadvantage: None,
                expertise: false,
            },
            SheetSkill {
                key: "arcana".to_owned(),
                name: "Arcana".to_owned(),
                ability: "Intelligence".to_owned(),
                bonus: -1,
                source: None,
                disadvantage: None,
                expertise: false,
            },
        ],
        attacks: Vec::new(),
        features: Vec::new(),
        feats: vec!["Alert (Human Versatile)".to_owned()],
        languages: vec!["Common".to_owned()],
        tools: Vec::new(),
        equipment: vec!["Chain Mail".to_owned()],
        coins_gp: 18,
        current_hp: Some(9),
        temp_hp: 0,
        hit_dice: Some("1 of 1 d10".to_owned()),
        uses: Vec::new(),
        spellcasting: None,
        marks: vec![SheetMark {
            subject: "Weapon Mastery".to_owned(),
            note: "Fighter: Weapon Mastery is not in play yet.".to_owned(),
        }],
    }
}

fn view() -> CreationView {
    CreationView {
        state: BuildState::Reviewing,
        hero: Some(BuildHero {
            name: "Mara".to_owned(),
            species: "human".to_owned(),
            class: "fighter".to_owned(),
            portrait: None,
        }),
        picks: BTreeMap::from([
            ("background".to_owned(), "soldier".to_owned()),
            ("ability-method".to_owned(), "standard-array".to_owned()),
            ("strength".to_owned(), "15".to_owned()),
            ("dexterity".to_owned(), "14".to_owned()),
            ("class-skill-1".to_owned(), "survival".to_owned()),
        ]),
        groups: vec![
            BuildGroup {
                options: vec![
                    option("soldier", "Soldier"),
                    BuildOption {
                        not_in_play: Some("Not printed in the SRD.".to_owned()),
                        ..option("farmer", "Farmer")
                    },
                ],
                ..group("background", &["background"], Vec::new())
            },
            group(
                "ability-scores",
                &["strength", "dexterity"],
                vec![option("15", "15"), option("14", "14")],
            ),
            group(
                "class-skills",
                &["class-skill-1", "class-skill-2"],
                vec![option("survival", "Survival"), option("history", "History")],
            ),
        ],
        issues: Vec::new(),
        sheet: Some(sheet()),
    }
}

fn panels_of(view: &CreationView, ui: &BuildUi) -> Vec<Block> {
    build_panels(&BuildInput {
        view: Some(view),
        ui,
        scene: None,
    })
    .blocks
}

fn chips(blocks: &[Block], label: &str) -> Vec<Chip> {
    blocks
        .iter()
        .find_map(|block| match block {
            Block::Chips {
                label: found,
                chips,
            } if found.starts_with(label) => Some(chips.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[test]
fn a_build_tap_reads_back_from_its_action() {
    for tap in [
        BuildTap::Open(None),
        BuildTap::Open(Some(BuildPart::Skills)),
        BuildTap::Surprise,
        BuildTap::Submit,
        BuildTap::Pick {
            set: vec![
                ("strength".to_owned(), "14".to_owned()),
                ("dexterity".to_owned(), "15".to_owned()),
            ],
            clear: vec!["class-skill-2".to_owned()],
        },
    ] {
        assert_eq!(BuildTap::from_action(&tap.action()), Some(tap));
    }
    assert_eq!(BuildTap::from_action("pages:bag"), None);
    assert_eq!(BuildTap::from_action("build:open-cooking"), None);
}

#[test]
fn the_review_sheet_shows_the_numbers_each_part_to_change_the_marks_and_ready() {
    let blocks = panels_of(&view(), &BuildUi::default());
    assert!(blocks.iter().any(|block| matches!(
        block,
        Block::Stats { stats }
            if stats.iter().any(|stat| stat.label == "Armor Class" && stat.value == "17")
    )));
    let rows = blocks
        .iter()
        .find_map(|block| match block {
            Block::Rows { rows } => Some(rows.clone()),
            _ => None,
        })
        .expect("the parts are lines");
    let background = &rows[0];
    assert_eq!(background.value.as_deref(), Some("Soldier"));
    assert_eq!(
        background.action.as_deref(),
        Some(
            BuildTap::Open(Some(BuildPart::Background))
                .action()
                .as_str()
        )
    );
    assert!(
        rows.iter()
            .any(|row| row.label == "Skills and tools" && row.mark.is_some())
    );
    assert!(blocks.iter().any(|block| matches!(
        block,
        Block::Notice { text, .. } if text.starts_with("Not in play yet: Fighter")
    )));
    let buttons = blocks
        .iter()
        .find_map(|block| match block {
            Block::Buttons { buttons } => Some(buttons.clone()),
            _ => None,
        })
        .expect("the review ends with its buttons");
    assert_eq!(buttons[0].action, Some(BuildTap::Submit.action()));
    assert!(buttons[0].enabled);
}

#[test]
fn a_design_only_background_is_offered_with_its_mark_and_the_chosen_one_is_lit() {
    let ui = BuildUi {
        part: Some(BuildPart::Background),
        ..BuildUi::default()
    };
    let blocks = panels_of(&view(), &ui);
    let cards = blocks
        .iter()
        .find_map(|block| match block {
            Block::Cards { cards, .. } => Some(cards.clone()),
            _ => None,
        })
        .expect("backgrounds are cards");
    assert!(cards[0].selected);
    assert!(cards[1].mark.is_some() && !cards[1].off);
    assert_eq!(
        cards[1].action,
        Some(
            BuildTap::Pick {
                set: vec![("background".to_owned(), "farmer".to_owned())],
                clear: Vec::new(),
            }
            .action()
        )
    );
}

#[test]
fn a_standard_array_score_taken_from_another_ability_swaps_the_two() {
    let ui = BuildUi {
        part: Some(BuildPart::Abilities),
        ..BuildUi::default()
    };
    let blocks = panels_of(&view(), &ui);
    let strength = chips(&blocks, "Strength");
    assert_eq!(
        strength[1].action,
        Some(
            BuildTap::Pick {
                set: vec![
                    ("strength".to_owned(), "14".to_owned()),
                    ("dexterity".to_owned(), "15".to_owned()),
                ],
                clear: Vec::new(),
            }
            .action()
        )
    );
}

#[test]
fn a_skill_chip_fills_the_first_empty_slot_and_a_chosen_one_is_taken_back() {
    let ui = BuildUi {
        part: Some(BuildPart::Skills),
        ..BuildUi::default()
    };
    let blocks = panels_of(&view(), &ui);
    let skills = chips(&blocks, "class-skills");
    assert!(skills[0].selected);
    assert_eq!(
        skills[0].action,
        Some(
            BuildTap::Pick {
                set: Vec::new(),
                clear: vec!["class-skill-1".to_owned()],
            }
            .action()
        )
    );
    assert_eq!(
        skills[1].action,
        Some(
            BuildTap::Pick {
                set: vec![("class-skill-2".to_owned(), "history".to_owned())],
                clear: Vec::new(),
            }
            .action()
        )
    );
}

#[test]
fn the_build_waits_for_the_host_before_it_shows_anything() {
    let panels = build_panels(&BuildInput {
        view: None,
        ui: &BuildUi::default(),
        scene: Some("room.png"),
    });
    assert!(panels.loading.is_some());
    assert_eq!(panels.scene.as_deref(), Some("room.png"));
}

#[test]
fn the_hero_and_roll_pages_draw_the_sheet_and_say_so_without_one() {
    let pages = SeatPages {
        sheet: Some(sheet()),
        ..SeatPages::default()
    };
    let hero = hero_page(&pages, "Mara", None);
    assert!(hero.blocks.iter().any(|block| matches!(
        block,
        Block::Stats { stats }
            if stats.iter().any(|stat| stat.label == "Hit points" && stat.value == "9 / 12")
    )));
    assert!(hero.blocks.iter().any(|block| matches!(
        block,
        Block::Notice { text, .. } if text.contains("Weapon Mastery")
    )));
    let roll = roll_page(&pages, None, None);
    let first = roll
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::Rows { rows } => rows.first().cloned(),
            _ => None,
        })
        .expect("the checks are lines");
    assert_eq!(
        (first.label.as_str(), first.value.as_deref()),
        ("Athletics", Some("+5"))
    );
    assert!(
        hero_page(&SeatPages::default(), "Mara", None)
            .empty
            .is_some()
    );
}

#[test]
fn a_roll_tap_names_its_check_and_reads_back() {
    use crate::seat::wire::SeatCheck;
    for check in [
        SeatCheck::Skill {
            skill: "animal_handling".to_owned(),
        },
        SeatCheck::Save {
            ability: "str".to_owned(),
        },
        SeatCheck::Ability {
            ability: "dex".to_owned(),
        },
    ] {
        assert_eq!(
            super::sheet::roll_of(&super::sheet::roll_action(&check)),
            Some(check)
        );
    }
    assert_eq!(super::sheet::roll_of("roll:skill:"), None);
    assert_eq!(super::sheet::roll_of("build:open"), None);
}

#[test]
fn every_check_on_the_roll_page_is_tappable_and_the_answer_shows_above_them() {
    use super::sheet::{RollShown, roll_action};
    use crate::fixture::RollOutcome;
    use crate::seat::wire::{SeatCheck, SeatRolled};
    let pages = SeatPages {
        sheet: Some(sheet()),
        ..SeatPages::default()
    };
    let shown = RollShown::Rolled {
        check: "Athletics check".to_owned(),
        rolled: SeatRolled::Rolled {
            roll: 12,
            modifier: 5,
            total: 17,
            dc: Some(15),
            outcome: Some(RollOutcome::Success),
        },
    };
    let panels = roll_page(&pages, Some(&shown), None);
    assert!(panels.blocks.iter().any(|block| matches!(
        block,
        Block::Stats { stats }
            if stats[0].sub.as_deref() == Some("12 + 5 = 17 against 15")
    )));
    assert!(panels.blocks.iter().any(|block| matches!(
        block,
        Block::Notice { text, .. } if text == "Success"
    )));
    let rows: Vec<_> = panels
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Rows { rows } => Some(rows.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(rows.iter().all(|row| row.action.is_some()));
    assert_eq!(
        rows[0].action,
        Some(roll_action(&SeatCheck::Skill {
            skill: "athletics".to_owned()
        }))
    );
    assert!(rows.iter().any(|row| row.action
        == Some(roll_action(&SeatCheck::Save {
            ability: "str".to_owned()
        }))));
    let told = RollShown::Rolled {
        check: "Arcana check".to_owned(),
        rolled: SeatRolled::NotCalled {
            reason: "nothing_to_learn".to_owned(),
        },
    };
    assert!(
        roll_page(&pages, Some(&told), None)
            .blocks
            .iter()
            .any(|block| matches!(
                block,
                Block::Notice { text, .. } if text.contains("nothing to learn")
            ))
    );
}

/// Every line of the Roll page, in order.
fn roll_rows(panels: &crate::phone_kit::PhonePanels) -> Vec<crate::phone_kit::page::Row> {
    panels
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Rows { rows } => Some(rows.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn the_called_roll_leads_the_roll_page_and_marks_its_check_with_the_act_pages_roll() {
    use crate::fixture::{RollMode, RollTarget};
    use crate::seat::screen::SeatTap;
    use crate::seat::wire::PendingRoll;
    let pages = SeatPages {
        sheet: Some(sheet()),
        ..SeatPages::default()
    };
    let called = |kind: &str, ability: Option<&str>, skill: Option<&str>| PendingRoll {
        roll_id: "roll-7".to_owned(),
        title: "Athletics, Strength".to_owned(),
        kind: kind.to_owned(),
        ability: ability.map(str::to_owned),
        skill: skill.map(str::to_owned),
        mode: RollMode::Disadvantage,
        target: Some(RollTarget {
            label: "DC".to_owned(),
            value: 15,
        }),
    };
    let marked = |rows: &[crate::phone_kit::page::Row]| {
        rows.iter()
            .filter(|row| row.mark.as_ref().is_some_and(|mark| mark.text == "Called"))
            .map(|row| (row.label.clone(), row.action.clone()))
            .collect::<Vec<_>>()
    };
    let roll = Some(SeatTap::Roll.action());

    let skill = called("ability_check", Some("strength"), Some("athletics"));
    let rows = roll_rows(&roll_page(&pages, None, Some(&skill)));
    assert_eq!(
        (
            rows[0].label.as_str(),
            rows[0].value.as_deref(),
            rows[0].because.as_deref()
        ),
        ("Athletics, Strength", Some("DC 15"), Some("Disadvantage"))
    );
    assert_eq!(
        marked(&rows),
        [
            ("Athletics, Strength".to_owned(), roll.clone()),
            ("Athletics".to_owned(), roll.clone())
        ]
    );

    let save = called("saving_throw", Some("strength"), None);
    let rows = roll_rows(&roll_page(&pages, None, Some(&save)));
    assert_eq!(marked(&rows).len(), 2, "the call and the Strength save");
    assert!(
        rows.iter()
            .filter(|row| row.label == "Strength")
            .any(|row| row.action == roll && row.value.as_deref() == Some("+5"))
    );

    // An attack is no check of the sheet's: only the call's own line is marked.
    let attack = called("melee_attack", None, None);
    assert_eq!(
        marked(&roll_rows(&roll_page(&pages, None, Some(&attack)))).len(),
        1
    );
    assert!(marked(&roll_rows(&roll_page(&pages, None, None))).is_empty());
}

#[test]
fn a_skill_the_worn_armour_hampers_shows_its_disadvantage() {
    let mut hampered = sheet();
    hampered.skills[0].disadvantage = Some("Chain Mail".to_owned());
    let pages = SeatPages {
        sheet: Some(hampered),
        ..SeatPages::default()
    };
    let rows = roll_rows(&roll_page(&pages, None, None));
    let athletics = rows
        .iter()
        .find(|row| row.label == "Athletics")
        .expect("the skill's line");
    assert_eq!(
        (
            athletics.mark.as_ref().map(|mark| mark.text.as_str()),
            athletics.because.as_deref()
        ),
        (
            Some("Disadvantage"),
            Some("Strength · Disadvantage (Chain Mail)")
        )
    );
}

#[test]
fn the_hero_page_lists_spells_with_cast_slots_left_and_marks_expertise() {
    use super::screens::PageTap;
    use super::wire::{SheetSlots, SheetSpell, SheetSpellcasting};
    let spell = |id: &str, level: u8, ritual: bool, prepared: bool| SheetSpell {
        id: id.to_owned(),
        name: id.to_owned(),
        level,
        school: "Divination".to_owned(),
        ritual,
        concentration: false,
        prepared,
        casting_time: "Action".to_owned(),
        range: "Self".to_owned(),
        duration: "Instantaneous".to_owned(),
        summary: String::new(),
    };
    let mut sheet = sheet();
    sheet.skills[0].expertise = true;
    sheet.spellcasting = Some(SheetSpellcasting {
        ability: "Intelligence".to_owned(),
        save_dc: 13,
        attack_bonus: 5,
        prepared_max: 4,
        spells: vec![
            spell("detect_magic", 1, true, false),
            spell("light", 0, false, true),
            spell("sleep", 1, false, false),
        ],
        slots: vec![SheetSlots {
            level: 1,
            left: 0,
            maximum: 2,
        }],
    });
    let pages = SeatPages {
        sheet: Some(sheet),
        ..SeatPages::default()
    };
    let hero = hero_page(&pages, "Mara", Some("No slot of that level is left."));
    let rows: Vec<_> = hero
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Rows { rows } => Some(rows.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let row = |label: &str| {
        rows.iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("{label} is listed"))
    };
    assert_eq!(
        row("Athletics")
            .mark
            .as_ref()
            .map(|mark| mark.text.as_str()),
        Some("Expertise")
    );
    // A cantrip is cast at level 0; an unprepared ritual is cast only as a ritual, from its own
    // line.
    assert_eq!(row("light").value.as_deref(), Some("Cantrip"));
    assert_eq!(
        row("light").action,
        Some(
            PageTap::Cast {
                spell: "light".to_owned(),
                level: 0,
                ritual: false,
            }
            .action()
        )
    );
    assert_eq!(
        row("detect_magic")
            .mark
            .as_ref()
            .map(|mark| mark.text.as_str()),
        Some("Ritual")
    );
    assert!(row("detect_magic").action.is_none());
    assert_eq!(
        row("detect_magic as a ritual").action,
        Some(
            PageTap::Cast {
                spell: "detect_magic".to_owned(),
                level: 1,
                ritual: true,
            }
            .action()
        )
    );
    assert!(row("sleep").action.is_none());
    let slots = hero.blocks.iter().find_map(|block| match block {
        Block::Stats { stats } => stats.iter().find(|stat| stat.label == "Level 1 slots"),
        _ => None,
    });
    let slots = slots.expect("the slots are shown");
    assert_eq!(slots.value, "0 / 2");
    assert!(slots.spent);
    assert!(hero.blocks.iter().any(|block| matches!(
        block,
        Block::Notice { text, .. } if text == "No slot of that level is left."
    )));
    for ritual in [false, true] {
        let tap = PageTap::Cast {
            spell: "magic_missile".to_owned(),
            level: 1,
            ritual,
        };
        assert_eq!(PageTap::from_action(&tap.action()), Some(tap));
    }
}

#[test]
fn class_picks_open_as_their_own_part_and_a_spellbook_reads_one_spell_to_a_line() {
    let spell = |id: &str, detail: &str| BuildOption {
        detail: Some(detail.to_owned()),
        ..option(id, id)
    };
    let mut view = view();
    view.groups.push(group(
        "spellbook",
        &["spellbook-1", "spellbook-2"],
        vec![
            spell("alarm", "Level 1 · Abjuration · ritual · A ward."),
            spell("sleep", "Level 1 · Enchantment · Creatures fall asleep."),
        ],
    ));
    view.picks
        .insert("spellbook-1".to_owned(), "sleep".to_owned());
    view.groups.push(group(
        "prepared-spells",
        &["prepared-1"],
        vec![spell("alarm", "A ward."), spell("sleep", "Sleep.")],
    ));
    view.picks
        .insert("prepared-1".to_owned(), "sleep".to_owned());
    let review = panels_of(&view, &BuildUi::default());
    // A prepared spell is a spellbook spell: each list under its own label, never one list that
    // names it twice (F58).
    let class_line = review
        .iter()
        .find_map(|block| match block {
            Block::Rows { rows } => rows
                .iter()
                .find(|row| row.label == "Class features")
                .and_then(|row| row.value.clone()),
            _ => None,
        })
        .expect("the class line");
    assert_eq!(class_line, "spellbook: sleep; prepared-spells: sleep");
    let part = panels_of(
        &view,
        &BuildUi {
            part: Some(BuildPart::Class),
            ..BuildUi::default()
        },
    );
    let rows = part
        .iter()
        .find_map(|block| match block {
            Block::Rows { rows } => Some(rows),
            _ => None,
        })
        .expect("the spellbook is a list");
    assert_eq!(
        rows[0].because.as_deref(),
        Some("Level 1 · Abjuration · ritual · A ward.")
    );
    // The free slot takes the new pick; the chosen one is taken back.
    assert_eq!(
        rows[0].action,
        Some(
            BuildTap::Pick {
                set: vec![("spellbook-2".to_owned(), "alarm".to_owned())],
                clear: Vec::new(),
            }
            .action()
        )
    );
    assert_eq!(
        rows[1].mark.as_ref().map(|mark| mark.text.as_str()),
        Some("Chosen")
    );
    assert_eq!(
        rows[1].action,
        Some(
            BuildTap::Pick {
                set: Vec::new(),
                clear: vec!["spellbook-1".to_owned()],
            }
            .action()
        )
    );
}

#[test]
fn a_full_multi_pick_group_never_replaces_a_pick_and_says_how_many_are_chosen() {
    let cantrip = |id: &str| BuildOption {
        detail: Some(format!("Cantrip · {id}")),
        ..option(id, id)
    };
    let mut view = view();
    view.groups.push(group(
        "cantrips",
        &["cantrip-1", "cantrip-2", "cantrip-3"],
        vec![
            cantrip("light"),
            cantrip("mage-hand"),
            cantrip("prestidigitation"),
            cantrip("ray-of-frost"),
        ],
    ));
    let ui = BuildUi {
        part: Some(BuildPart::Class),
        ..BuildUi::default()
    };
    let caption = |blocks: &[Block]| {
        blocks
            .iter()
            .find_map(|block| match block {
                Block::Caption { text } if text.contains("chosen") => Some(text.clone()),
                _ => None,
            })
            .expect("the group has a header")
    };
    let rows = |blocks: &[Block]| {
        blocks
            .iter()
            .find_map(|block| match block {
                Block::Rows { rows } => Some(rows.clone()),
                _ => None,
            })
            .expect("the cantrips are a list")
    };
    // Two of three chosen, the middle slot empty: a pick fills that slot, not the last.
    view.picks
        .insert("cantrip-1".to_owned(), "mage-hand".to_owned());
    view.picks
        .insert("cantrip-3".to_owned(), "prestidigitation".to_owned());
    let blocks = panels_of(&view, &ui);
    assert_eq!(caption(&blocks), "cantrips: 2 of 3 chosen");
    assert_eq!(
        rows(&blocks)[0].action,
        Some(
            BuildTap::Pick {
                set: vec![("cantrip-2".to_owned(), "light".to_owned())],
                clear: Vec::new(),
            }
            .action()
        )
    );
    // Full: no pick replaces another; a chosen one is taken back from exactly its slot.
    view.picks
        .insert("cantrip-2".to_owned(), "ray-of-frost".to_owned());
    let blocks = panels_of(&view, &ui);
    assert_eq!(
        caption(&blocks),
        "cantrips: 3 of 3 chosen — take one back to change"
    );
    let full = rows(&blocks);
    assert_eq!(
        (full[0].action.as_ref(), full[0].change.as_ref()),
        (None, None)
    );
    assert_eq!(
        full[2].action,
        Some(
            BuildTap::Pick {
                set: Vec::new(),
                clear: vec!["cantrip-3".to_owned()],
            }
            .action()
        )
    );
    // A full group of chips offers no tap on an unchosen chip either.
    view.picks
        .insert("class-skill-2".to_owned(), "history".to_owned());
    view.picks
        .insert("class-skill-1".to_owned(), "survival".to_owned());
    let mut skills = view.clone();
    skills.groups[2].options.push(option("arcana", "Arcana"));
    let skill_ui = BuildUi {
        part: Some(BuildPart::Skills),
        ..BuildUi::default()
    };
    let chips = chips(&panels_of(&skills, &skill_ui), "class-skills");
    assert!(chips[2].action.is_none());
}

#[test]
fn the_last_roll_is_a_full_card_only_while_the_screen_shows_it_and_nothing_is_asked() {
    use super::sheet::{roll_card_up, roll_summary};
    use crate::fixture::{RollTarget, RollView};
    let roll: RollView = serde_json::from_value(serde_json::json!({
        "id": "roll-7",
        "title": "History",
        "kind": "skill_check",
        "dice": [{"sides": 20, "face": 15, "role": "test"}],
        "modifiers": [{"label": "INT", "value": 2}],
        "total": 17,
        "outcome": "success"
    }))
    .expect("a roll");
    let roll = RollView {
        target: Some(RollTarget {
            label: "DC".to_owned(),
            value: 10,
        }),
        ..roll
    };
    assert!(roll_card_up(false, &roll, Some("roll-7")));
    assert!(
        !roll_card_up(true, &roll, Some("roll-7")),
        "a new question comes first"
    );
    assert!(
        !roll_card_up(false, &roll, None),
        "the screen let the card go"
    );
    assert!(!roll_card_up(false, &roll, Some("roll-8")));
    assert_eq!(roll_summary(&roll), "History 17 vs DC 10: success");
}
