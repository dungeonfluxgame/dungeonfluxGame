//! The Hero and Roll pages in play: the hero's sheet as the rules derived it (P2), and the checks
//! the sheet offers.
//!
//! Every number is the host's (`HeroSheetView`). The Roll page lists what each check adds; a tap
//! asks the table to roll it (`path::PLAY_SEAT_ROLL`): the check judge decides whether it is called
//! and against what, and the host rolls. The page shows the answer; it never rolls. The roll the
//! dungeon master calls for (`SeatView::pending_roll`) leads the page, and its check is marked;
//! their Roll is the Act page's (`SeatTap::Roll`).

use super::screens::PageTap;
use super::wire::{HeroSheetView, SeatPages, SheetSkill, SheetSpell, SheetSpellcasting};
use crate::fixture::RollKind;
use crate::kit::roll_card::{Tone, mode_word, outcome_word};
use crate::phone_kit::PhonePanels;
use crate::phone_kit::page::{Block, Chip, Mark, Row, Stat};
use crate::phone_kit::shell::{EmptyState, PillTone};
use crate::seat::screen::SeatTap;
use crate::seat::wire::{PendingRoll, SeatCheck, SeatRolled};

/// The Hero page.
pub const HERO_PAGE: &str = "hero";
/// The Roll page.
pub const ROLL_PAGE: &str = "roll";

fn signed(value: i64) -> String {
    format!("{value:+}")
}

fn stat(label: &str, value: String, sub: Option<String>) -> Stat {
    Stat {
        label: label.to_owned(),
        value,
        sub,
        spent: false,
    }
}

fn no_sheet() -> PhonePanels {
    PhonePanels {
        empty: Some(EmptyState {
            title: "No sheet yet".to_owned(),
            text: "Your hero's sheet comes with the hero you build after Lock in.".to_owned(),
            glyph: None,
        }),
        ..PhonePanels::default()
    }
}

fn row(label: &str, value: String, because: Option<String>, mark: Option<Mark>) -> Row {
    Row {
        label: label.to_owned(),
        value: Some(value),
        because,
        change: None,
        mark,
        picture: None,
        action: None,
    }
}

fn trained() -> Option<Mark> {
    Some(Mark {
        text: "Trained".to_owned(),
        tone: PillTone::Good,
    })
}

/// A skill's mark: Expertise (double proficiency) over Trained.
fn skill_mark(skill: &SheetSkill) -> Option<Mark> {
    if skill.expertise {
        Some(Mark {
            text: "Expertise".to_owned(),
            tone: PillTone::Accent,
        })
    } else {
        skill.source.as_ref().and_then(|_| trained())
    }
}

/// The hero's sheet: the numbers, the abilities and saves, the skills, the attacks, the spells,
/// what the hero has and knows, and what is not in play yet. `problem` is why the table did not
/// take the last cast.
pub fn hero_page(pages: &SeatPages, hero: &str, problem: Option<&str>) -> PhonePanels {
    let Some(sheet) = &pages.sheet else {
        return no_sheet();
    };
    let mut blocks = vec![
        Block::Heading {
            text: hero.to_owned(),
            caption: Some(format!(
                "{} {} {}, level {} · {}",
                sheet.species, sheet.class, sheet.background, sheet.level, sheet.alignment
            )),
        },
        Block::Stats {
            stats: vec![
                stat(
                    "Hit points",
                    match sheet.current_hp {
                        Some(current) => format!("{current} / {}", sheet.hit_points),
                        None => sheet.hit_points.to_string(),
                    },
                    (sheet.temp_hp > 0).then(|| format!("+{} temporary", sheet.temp_hp)),
                ),
                stat(
                    "Armor Class",
                    sheet.armor_class.to_string(),
                    Some(sheet.armor_note.clone()),
                ),
                stat("Initiative", signed(i64::from(sheet.initiative)), None),
                stat(
                    "Speed",
                    sheet
                        .speed_feet
                        .map_or_else(|| "Not in play yet".to_owned(), |feet| format!("{feet} ft")),
                    None,
                ),
                stat(
                    "Proficiency",
                    signed(i64::from(sheet.proficiency_bonus)),
                    None,
                ),
                stat(
                    "Passive Perception",
                    sheet.passive_perception.to_string(),
                    None,
                ),
            ],
        },
        Block::Stats {
            stats: sheet
                .abilities
                .iter()
                .map(|ability| {
                    stat(
                        &ability.key.to_uppercase(),
                        ability.score.to_string(),
                        Some(format!(
                            "{} · save {}{}",
                            signed(i64::from(ability.modifier)),
                            signed(i64::from(ability.save)),
                            if ability.save_proficient {
                                ", trained"
                            } else {
                                ""
                            }
                        )),
                    )
                })
                .collect(),
        },
        Block::Caption {
            text: "Skills".to_owned(),
        },
        Block::Rows {
            rows: sheet
                .skills
                .iter()
                .map(|skill| {
                    row(
                        &skill.name,
                        signed(i64::from(skill.bonus)),
                        skill.source.clone(),
                        skill_mark(skill),
                    )
                })
                .collect(),
        },
    ];
    if !sheet.attacks.is_empty() {
        blocks.push(Block::Caption {
            text: "Attacks".to_owned(),
        });
        blocks.push(Block::Rows {
            rows: sheet
                .attacks
                .iter()
                .map(|attack| {
                    row(
                        &attack.name,
                        format!("{} to hit", signed(i64::from(attack.bonus))),
                        Some(format!("{} {}", attack.damage, attack.damage_type)),
                        None,
                    )
                })
                .collect(),
        });
    }
    if let Some(spellcasting) = &sheet.spellcasting {
        spells(spellcasting, problem, &mut blocks);
    }
    features(sheet, &mut blocks);
    PhonePanels {
        blocks,
        ..PhonePanels::default()
    }
}

/// The level words of a spell: a cantrip, or its level.
fn level_words(level: u8) -> String {
    if level == 0 {
        "Cantrip".to_owned()
    } else {
        format!("Level {level}")
    }
}

/// A spell as lines: its level, how it is cast, and Cast on a cantrip or a prepared spell; a
/// ritual has a second line to cast it as a ritual (longer, and with no slot, as the host words
/// it). The host decides whether a cast is allowed and checks the slot; the page only offers the
/// taps.
fn spell_rows(spell: &SheetSpell) -> Vec<Row> {
    let mut how = vec![
        spell.school.clone(),
        spell.casting_time.clone(),
        spell.range.clone(),
        spell.duration.clone(),
    ];
    if spell.concentration {
        how.push("Concentration".to_owned());
    }
    if !spell.prepared {
        how.push("Not prepared".to_owned());
    }
    let mut because = how
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if !spell.summary.is_empty() {
        because = format!("{because}. {}", spell.summary);
    }
    let cast = |ritual: bool| {
        PageTap::Cast {
            spell: spell.id.clone(),
            level: spell.level,
            ritual,
        }
        .action()
    };
    let mut rows = vec![Row {
        label: spell.name.clone(),
        value: Some(level_words(spell.level)),
        because: Some(because),
        change: spell.prepared.then(|| "Cast".to_owned()),
        mark: spell.ritual.then(|| Mark {
            text: "Ritual".to_owned(),
            tone: PillTone::Accent,
        }),
        picture: None,
        action: spell.prepared.then(|| cast(false)),
    }];
    if spell.ritual {
        rows.push(Row {
            label: format!("{} as a ritual", spell.name),
            value: None,
            because: Some("Takes 10 minutes longer and uses no spell slot.".to_owned()),
            change: Some("Cast as ritual".to_owned()),
            mark: None,
            picture: None,
            action: Some(cast(true)),
        });
    }
    rows
}

/// The spells: the save DC, the attack bonus, the prepared count and the slots left, then the
/// cantrips and spells by level, each with Cast. `problem` is why the last cast was refused.
fn spells(spellcasting: &SheetSpellcasting, problem: Option<&str>, blocks: &mut Vec<Block>) {
    let prepared = spellcasting
        .spells
        .iter()
        .filter(|spell| spell.level > 0 && spell.prepared)
        .count();
    blocks.push(Block::Caption {
        text: "Spells".to_owned(),
    });
    if let Some(problem) = problem {
        blocks.push(Block::Notice {
            tone: PillTone::Warn,
            text: problem.to_owned(),
        });
    }
    let mut stats = vec![
        stat(
            "Spell save DC",
            spellcasting.save_dc.to_string(),
            Some(spellcasting.ability.clone()),
        ),
        stat(
            "Spell attack",
            signed(i64::from(spellcasting.attack_bonus)),
            None,
        ),
        stat(
            "Prepared",
            format!("{prepared} / {}", spellcasting.prepared_max),
            None,
        ),
    ];
    stats.extend(spellcasting.slots.iter().map(|slots| Stat {
        label: format!("Level {} slots", slots.level),
        value: format!("{} / {}", slots.left, slots.maximum),
        sub: Some("left".to_owned()),
        spent: slots.left == 0,
    }));
    blocks.push(Block::Stats { stats });
    let mut known: Vec<&SheetSpell> = spellcasting.spells.iter().collect();
    known.sort_by_key(|spell| (spell.level, spell.name.clone()));
    blocks.push(Block::Rows {
        rows: known.into_iter().flat_map(spell_rows).collect(),
    });
}

/// The features, feats, languages, tools, equipment and the marks of what is not in play yet.
fn features(sheet: &HeroSheetView, blocks: &mut Vec<Block>) {
    if !sheet.features.is_empty() {
        blocks.push(Block::Caption {
            text: "Features".to_owned(),
        });
        blocks.push(Block::Rows {
            rows: sheet
                .features
                .iter()
                .map(|feature| {
                    // The sheet says when a feature waits for combat (Sneak Attack).
                    let waits = feature.status.as_deref() == Some("applies_in_combat");
                    row(
                        &feature.name,
                        String::new(),
                        Some(feature.text.clone()),
                        waits.then(|| Mark {
                            text: "In combat".to_owned(),
                            tone: PillTone::Neutral,
                        }),
                    )
                })
                .collect(),
        });
    }
    for (label, items) in [
        ("Feats", &sheet.feats),
        ("Languages", &sheet.languages),
        ("Tools", &sheet.tools),
        ("Equipment", &sheet.equipment),
    ] {
        if items.is_empty() {
            continue;
        }
        blocks.push(Block::Chips {
            label: label.to_owned(),
            chips: items
                .iter()
                .map(|item| Chip {
                    text: item.clone(),
                    selected: false,
                    removable: false,
                    action: None,
                })
                .collect(),
        });
    }
    if let Some(hit_dice) = &sheet.hit_dice {
        blocks.push(Block::Paragraph {
            text: format!("Hit Point Dice: {hit_dice}"),
        });
    }
    if !sheet.uses.is_empty() {
        blocks.push(Block::Stats {
            stats: sheet
                .uses
                .iter()
                .map(|uses| Stat {
                    label: uses.name.clone(),
                    value: format!("{} / {}", uses.left, uses.maximum),
                    sub: Some("uses left".to_owned()),
                    spent: uses.left == 0,
                })
                .collect(),
        });
    }
    blocks.push(Block::Paragraph {
        text: format!("Coins: {} GP", sheet.coins_gp),
    });
    for mark in &sheet.marks {
        blocks.push(Block::Notice {
            tone: PillTone::Neutral,
            text: format!("Not in play yet: {}", mark.note),
        });
    }
}

/// Whether the seat's last roll `last` shows as its full card on Act: while the shared screen
/// still shows it (`tv_roll`, the id of the roll the screen shows) and the table asks nothing.
/// Once the table asks again, or the screen has let the card go, the question comes first.
pub fn roll_card_up(asked: bool, last: &crate::fixture::RollView, tv_roll: Option<&str>) -> bool {
    !asked && tv_roll == Some(last.id.as_str())
}

/// A roll in one line: what was tested, the total, what it was against and the verdict ("History
/// 17 vs DC 10: success").
pub fn roll_summary(roll: &crate::fixture::RollView) -> String {
    let mut line = format!("{} {}", roll.title, roll.total);
    if let Some(target) = &roll.target {
        line.push_str(&format!(" vs {} {}", target.label, target.value));
    }
    if let Some(outcome) = roll.outcome {
        line.push_str(&format!(": {}", outcome_word(outcome).0.to_lowercase()));
    }
    line
}

/// The prefix of a roll tap's action: `roll:skill:athletics`, `roll:save:str`.
const ROLL_PREFIX: &str = "roll:";

/// The action of a tap that asks the table to roll `check`.
pub fn roll_action(check: &SeatCheck) -> String {
    let (kind, key) = match check {
        SeatCheck::Ability { ability } => ("ability", ability),
        SeatCheck::Skill { skill } => ("skill", skill),
        SeatCheck::Save { ability } => ("save", ability),
    };
    format!("{ROLL_PREFIX}{kind}:{key}")
}

/// The check a roll tap's action names, or `None` when it is something else's.
pub fn roll_of(action: &str) -> Option<SeatCheck> {
    let (kind, key) = action.strip_prefix(ROLL_PREFIX)?.split_once(':')?;
    if key.is_empty() {
        return None;
    }
    let key = key.to_owned();
    match kind {
        "ability" => Some(SeatCheck::Ability { ability: key }),
        "skill" => Some(SeatCheck::Skill { skill: key }),
        "save" => Some(SeatCheck::Save { ability: key }),
        _ => None,
    }
}

/// The check's name as the sheet words it: "Athletics check", "Strength save".
pub fn check_label(pages: &SeatPages, check: &SeatCheck) -> String {
    let sheet = pages.sheet.as_ref();
    let ability_name = |key: &str| {
        sheet
            .and_then(|sheet| sheet.abilities.iter().find(|ability| ability.key == key))
            .map_or_else(|| key.to_uppercase(), |ability| ability.name.clone())
    };
    match check {
        SeatCheck::Ability { ability } => format!("{} check", ability_name(ability)),
        SeatCheck::Save { ability } => format!("{} save", ability_name(ability)),
        SeatCheck::Skill { skill } => {
            let name = sheet
                .and_then(|sheet| sheet.skills.iter().find(|known| known.key == *skill))
                .map_or_else(|| skill.replace('_', " "), |known| known.name.clone());
            format!("{name} check")
        }
    }
}

/// The roll the Roll page shows: the one asked for, on its way, rolled, or not called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollShown {
    Rolling {
        check: String,
    },
    Rolled {
        check: String,
        rolled: SeatRolled,
    },
    /// The table did not roll it; the player's words for why.
    Told {
        check: String,
        words: String,
    },
}

/// The words of a reason key the check judge gave: `not_in_scene` reads "not in scene".
fn reason_words(reason: &str) -> String {
    reason.replace('_', " ")
}

/// The shown roll as blocks: the sum and its verdict, or why there is none.
fn shown_blocks(shown: &RollShown) -> Vec<Block> {
    match shown {
        RollShown::Rolling { check } => vec![Block::Notice {
            tone: PillTone::Neutral,
            text: format!("{check}: the table is rolling."),
        }],
        RollShown::Told { check, words } => vec![Block::Notice {
            tone: PillTone::Warn,
            text: format!("{check}: {words}"),
        }],
        RollShown::Rolled { check, rolled } => match rolled {
            SeatRolled::NotCalled { reason } => vec![Block::Notice {
                tone: PillTone::Warn,
                text: format!(
                    "{check}: the table does not call for it now ({}).",
                    reason_words(reason)
                ),
            }],
            SeatRolled::Rolled {
                roll,
                modifier,
                total,
                dc,
                outcome,
            } => {
                let sign = if *modifier < 0 { '-' } else { '+' };
                let against = dc.map(|dc| format!(" against {dc}")).unwrap_or_default();
                let mut blocks = vec![Block::Stats {
                    stats: vec![stat(
                        check,
                        total.to_string(),
                        Some(format!(
                            "{roll} {sign} {} = {total}{against}",
                            modifier.unsigned_abs()
                        )),
                    )],
                }];
                if let Some(outcome) = outcome {
                    let (word, tone) = outcome_word(*outcome);
                    blocks.push(Block::Notice {
                        tone: match tone {
                            Tone::Positive => PillTone::Good,
                            Tone::Negative => PillTone::Warn,
                            Tone::Neutral => PillTone::Neutral,
                        },
                        text: word.to_owned(),
                    });
                }
                blocks
            }
        },
    }
}

/// The sheet's check the called roll `called` is, or `None` when it is no check of the sheet's
/// (an attack, initiative, damage).
fn called_check(sheet: &HeroSheetView, called: &PendingRoll) -> Option<SeatCheck> {
    let ability = || {
        let roll_key = called.ability.as_ref()?;
        sheet
            .abilities
            .iter()
            .find(|known| known.roll_key == *roll_key)
            .map(|known| known.key.clone())
    };
    match RollKind::from_key(&called.kind)? {
        RollKind::SavingThrow => Some(SeatCheck::Save {
            ability: ability()?,
        }),
        RollKind::AbilityCheck | RollKind::SkillCheck | RollKind::ContestedCheck => {
            match &called.skill {
                Some(skill) => Some(SeatCheck::Skill {
                    skill: skill.clone(),
                }),
                None => Some(SeatCheck::Ability {
                    ability: ability()?,
                }),
            }
        }
        _ => None,
    }
}

fn called_mark() -> Mark {
    Mark {
        text: "Called".to_owned(),
        tone: PillTone::Accent,
    }
}

/// The roll the dungeon master calls for, as the page's first line: its test, against what, and
/// Roll (the Act page's tap).
fn called_row(called: &PendingRoll) -> Row {
    Row {
        label: called.title.clone(),
        value: called
            .target
            .as_ref()
            .map(|target| format!("{} {}", target.label, target.value)),
        because: mode_word(called.mode).map(str::to_owned),
        change: Some("Roll".to_owned()),
        mark: Some(called_mark()),
        picture: None,
        action: Some(SeatTap::Roll.action()),
    }
}

/// A check as a line: what it adds, and Roll. The called check's Roll answers the call.
fn check_row(
    label: &str,
    bonus: i64,
    because: Option<String>,
    mark: Option<Mark>,
    check: &SeatCheck,
    called: Option<&SeatCheck>,
) -> Row {
    let is_called = called == Some(check);
    Row {
        label: label.to_owned(),
        value: Some(signed(bonus)),
        because,
        change: Some("Roll".to_owned()),
        mark: if is_called { Some(called_mark()) } else { mark },
        picture: None,
        action: Some(if is_called {
            SeatTap::Roll.action()
        } else {
            roll_action(check)
        }),
    }
}

/// The checks the sheet offers: each skill, ability and saving throw with what it adds, and Roll
/// to ask the table for it (the check judge sets the difficulty, the host rolls); the roll the
/// table calls for (`called`) and the last roll asked for above them.
pub fn roll_page(
    pages: &SeatPages,
    shown: Option<&RollShown>,
    called: Option<&PendingRoll>,
) -> PhonePanels {
    let Some(sheet) = &pages.sheet else {
        return no_sheet();
    };
    let called_check = called.and_then(|called| called_check(sheet, called));
    let called_check = called_check.as_ref();
    let mut skills: Vec<_> = sheet.skills.iter().collect();
    skills.sort_by_key(|skill| (std::cmp::Reverse(skill.bonus), skill.name.clone()));
    let mut blocks = vec![Block::Heading {
        text: "Your checks".to_owned(),
        caption: Some(
            "Tap Roll: the table sets what it takes, and the server rolls the dice.".to_owned(),
        ),
    }];
    if let Some(called) = called {
        blocks.push(Block::Rows {
            rows: vec![called_row(called)],
        });
    }
    blocks.extend(shown.map(shown_blocks).unwrap_or_default());
    blocks.push(Block::Rows {
        rows: skills
            .into_iter()
            .map(|skill| {
                let (because, mark) = match &skill.disadvantage {
                    Some(from) => (
                        format!("{} · Disadvantage ({from})", skill.ability),
                        Some(Mark {
                            text: "Disadvantage".to_owned(),
                            tone: PillTone::Warn,
                        }),
                    ),
                    None => (skill.ability.clone(), skill_mark(skill)),
                };
                check_row(
                    &skill.name,
                    i64::from(skill.bonus),
                    Some(because),
                    mark,
                    &SeatCheck::Skill {
                        skill: skill.key.clone(),
                    },
                    called_check,
                )
            })
            .collect(),
    });
    blocks.push(Block::Caption {
        text: "Ability checks".to_owned(),
    });
    blocks.push(Block::Rows {
        rows: sheet
            .abilities
            .iter()
            .map(|ability| {
                check_row(
                    &ability.name,
                    i64::from(ability.modifier),
                    None,
                    None,
                    &SeatCheck::Ability {
                        ability: ability.key.clone(),
                    },
                    called_check,
                )
            })
            .collect(),
    });
    blocks.push(Block::Caption {
        text: "Saving throws".to_owned(),
    });
    blocks.push(Block::Rows {
        rows: sheet
            .abilities
            .iter()
            .map(|ability| {
                check_row(
                    &ability.name,
                    i64::from(ability.save),
                    None,
                    ability.save_proficient.then(|| Mark {
                        text: "Trained".to_owned(),
                        tone: PillTone::Good,
                    }),
                    &SeatCheck::Save {
                        ability: ability.key.clone(),
                    },
                    called_check,
                )
            })
            .collect(),
    });
    PhonePanels {
        blocks,
        ..PhonePanels::default()
    }
}
