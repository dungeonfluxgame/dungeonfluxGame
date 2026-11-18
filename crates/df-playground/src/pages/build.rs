//! The phone's hero build after Lock in (`planning/phone-and-creation.md` P3): the quick hero
//! with its review sheet, and "Go deeper" into each part (background, ability scores, skills,
//! languages, equipment, the species' own choices), each with its one-line "because".
//!
//! Decided here from the host's `CreationView`, so native tests cover what each screen shows and
//! what each tap sends; the browser only draws it. A tap carries a `BuildTap` as its action. A
//! pick is a slot and an option id; a standard-array score taken from another ability swaps the
//! two; a multi-slot choice (skills, languages) fills its first empty slot. Every number shown is
//! the host's.

use super::creation::{BuildGroup, BuildState, CreationView};
use super::wire::HeroSheetView;
use crate::phone_kit::PhonePanels;
use crate::phone_kit::page::{Block, Chip, Mark, PickCard, Row, Stat};
use crate::phone_kit::shell::{ButtonKind, ButtonSpec, LoadingState, PhoneHeader, PillTone};

/// The prefix of a build tap's action.
const BUILD_PREFIX: &str = "build:";
/// The standard array's slots take each score once, so a score taken swaps (SRD p. 21).
const STANDARD_ARRAY: &str = "standard-array";
const ABILITY_METHOD: &str = "ability-method";
const ABILITY_SCORES: &str = "ability-scores";

/// A part of the build "Go deeper" opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildPart {
    Background,
    Abilities,
    Skills,
    Languages,
    Equipment,
    Species,
    /// What the class asks for: Expertise, weapon masteries, cantrips and a spellbook.
    Class,
}

impl BuildPart {
    pub const ALL: [Self; 7] = [
        Self::Background,
        Self::Abilities,
        Self::Skills,
        Self::Languages,
        Self::Equipment,
        Self::Species,
        Self::Class,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Abilities => "abilities",
            Self::Skills => "skills",
            Self::Languages => "languages",
            Self::Equipment => "equipment",
            Self::Species => "species",
            Self::Class => "class",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Background => "Background",
            Self::Abilities => "Ability scores",
            Self::Skills => "Skills and tools",
            Self::Languages => "Languages and outlook",
            Self::Equipment => "Equipment",
            Self::Species => "Your people",
            Self::Class => "Class features",
        }
    }

    /// The part a group of the build belongs to; none for the species and class the look studio
    /// chose.
    fn of(group: &str) -> Option<Self> {
        Some(match group {
            "species" | "class" => return None,
            "background" | "gaming-set" => Self::Background,
            ABILITY_METHOD | ABILITY_SCORES | "ability-increase" | "increases" => Self::Abilities,
            "class-skills" | "instruments" | "monk-tool" => Self::Skills,
            "languages" | "alignment" => Self::Languages,
            "class-equipment" | "background-equipment" | "fighting-style" => Self::Equipment,
            "expertise" | "weapon-mastery" | "rogue-language" | "cantrips" | "spellbook"
            | "prepared-spells" => Self::Class,
            _ => Self::Species,
        })
    }
}

/// A tap of the build screens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildTap {
    /// Open a part, or the review sheet (`None`).
    Open(Option<BuildPart>),
    /// Set these picks and clear these slots, then the rest is filled.
    Pick {
        set: Vec<(String, String)>,
        clear: Vec<String>,
    },
    Surprise,
    Submit,
}

impl BuildTap {
    pub fn action(&self) -> String {
        let body = match self {
            Self::Open(None) => "open".to_owned(),
            Self::Open(Some(part)) => format!("open-{}", part.key()),
            Self::Pick { set, clear } => {
                let set: Vec<String> = set
                    .iter()
                    .map(|(slot, option)| format!("{slot}={option}"))
                    .collect();
                format!("pick:{};{}", set.join(","), clear.join(","))
            }
            Self::Surprise => "surprise".to_owned(),
            Self::Submit => "submit".to_owned(),
        };
        format!("{BUILD_PREFIX}{body}")
    }

    /// The tap an action string names, or `None` when it is something else's.
    pub fn from_action(action: &str) -> Option<Self> {
        let body = action.strip_prefix(BUILD_PREFIX)?;
        match body {
            "open" => return Some(Self::Open(None)),
            "surprise" => return Some(Self::Surprise),
            "submit" => return Some(Self::Submit),
            _ => {}
        }
        if let Some(part) = body.strip_prefix("open-") {
            return BuildPart::ALL
                .into_iter()
                .find(|known| known.key() == part)
                .map(|part| Self::Open(Some(part)));
        }
        let (set, clear) = body.strip_prefix("pick:")?.split_once(';')?;
        let set = set
            .split(',')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                pair.split_once('=')
                    .map(|(slot, option)| (slot.to_owned(), option.to_owned()))
            })
            .collect::<Option<Vec<_>>>()?;
        let clear = clear
            .split(',')
            .filter(|slot| !slot.is_empty())
            .map(str::to_owned)
            .collect();
        Some(Self::Pick { set, clear })
    }
}

/// What the phone keeps of the build screens: the part open, whether a step is on its way and the
/// last refusal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildUi {
    pub part: Option<BuildPart>,
    pub busy: bool,
    pub problem: Option<String>,
}

/// Everything the build screens are drawn from.
#[derive(Debug, Clone, Copy)]
pub struct BuildInput<'a> {
    /// None until the host answered.
    pub view: Option<&'a CreationView>,
    pub ui: &'a BuildUi,
    /// The room's picture behind the screen.
    pub scene: Option<&'a str>,
}

fn button(label: &str, kind: ButtonKind, tap: &BuildTap, enabled: bool) -> ButtonSpec {
    ButtonSpec {
        label: label.to_owned(),
        kind,
        enabled,
        reason: None,
        action: Some(tap.action()),
    }
}

fn notice(tone: PillTone, text: &str) -> Block {
    Block::Notice {
        tone,
        text: text.to_owned(),
    }
}

fn signed(value: i64) -> String {
    format!("{value:+}")
}

/// The words for a title id: `high-elf` reads "High Elf".
fn titled(id: &str) -> String {
    id.split('-')
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(characters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The label of `option` in `group`, or the option id in words.
fn option_label(group: Option<&BuildGroup>, option: &str) -> String {
    group
        .and_then(|group| group.options.iter().find(|known| known.id == option))
        .map_or_else(|| titled(option), |known| known.label.clone())
}

fn group<'v>(view: &'v CreationView, id: &str) -> Option<&'v BuildGroup> {
    view.groups.iter().find(|group| group.id == id)
}

/// The picks of `group`, in slot order, as their labels.
fn picked(view: &CreationView, id: &str) -> Vec<String> {
    let Some(found) = group(view, id) else {
        return Vec::new();
    };
    found
        .slots
        .iter()
        .filter_map(|slot| view.picks.get(slot))
        .map(|option| option_label(Some(found), option))
        .collect()
}

/// The screen of the build: loading, the review sheet, or the part open.
pub fn build_panels(input: &BuildInput<'_>) -> PhonePanels {
    let Some(view) = input.view else {
        return PhonePanels {
            scene: input.scene.map(str::to_owned),
            loading: Some(LoadingState {
                text: "Getting your hero ready".to_owned(),
            }),
            ..PhonePanels::default()
        };
    };
    let header = view.hero.as_ref().map(|hero| PhoneHeader {
        name: hero.name.clone(),
        subtitle: Some(format!(
            "{} {}: your build",
            titled(&hero.species),
            titled(&hero.class)
        )),
        portrait: hero.portrait.clone(),
        hp: None,
    });
    let mut blocks = Vec::new();
    if let Some(problem) = &input.ui.problem {
        blocks.push(notice(PillTone::Warn, problem));
    }
    match input.ui.part {
        None => review(view, input.ui, &mut blocks),
        Some(part) => part_blocks(view, part, &mut blocks),
    }
    PhonePanels {
        scene: input.scene.map(str::to_owned),
        header,
        blocks,
        ..PhonePanels::default()
    }
}

/// The value of a review line, or what it waits for.
fn line_value(values: Vec<String>) -> String {
    if values.is_empty() {
        "Not chosen yet".to_owned()
    } else {
        values.join(", ")
    }
}

/// The review line of a part made of several groups: each group's picks under its own label
/// ("Spellbook: ...; Prepared spells (four from your spellbook): ..."), so a pick one group makes
/// from another's (a prepared spell is a spellbook spell) reads as what it is, never as the same
/// spell listed twice.
fn groups_line(view: &CreationView, part: BuildPart) -> String {
    let chosen: Vec<String> = view
        .groups
        .iter()
        .filter(|group| BuildPart::of(&group.id) == Some(part))
        .filter_map(|group| {
            let picks = picked(view, &group.id);
            (!picks.is_empty()).then(|| format!("{}: {}", group.label, picks.join(", ")))
        })
        .collect();
    if chosen.is_empty() {
        line_value(chosen)
    } else {
        chosen.join("; ")
    }
}

fn review_row(view: &CreationView, part: BuildPart, value: String, because: &str) -> Row {
    let waiting = view
        .groups
        .iter()
        .filter(|group| BuildPart::of(&group.id) == Some(part))
        .any(|group| {
            group
                .slots
                .iter()
                .any(|slot| !view.picks.contains_key(slot))
        });
    Row {
        label: part.title().to_owned(),
        value: Some(value),
        because: Some(because.to_owned()),
        change: Some("Change".to_owned()),
        mark: waiting.then(|| Mark {
            text: "To choose".to_owned(),
            tone: PillTone::Warn,
        }),
        picture: None,
        action: Some(BuildTap::Open(Some(part)).action()),
    }
}

fn scores_line(sheet: Option<&HeroSheetView>) -> String {
    sheet.map_or_else(
        || "Not chosen yet".to_owned(),
        |sheet| {
            sheet
                .abilities
                .iter()
                .map(|ability| format!("{} {}", ability.key.to_uppercase(), ability.score))
                .collect::<Vec<_>>()
                .join("  ")
        },
    )
}

/// The review sheet: the numbers, each part as a line to change, what is not in play yet, and
/// Surprise me and I'm ready.
fn review(view: &CreationView, ui: &BuildUi, blocks: &mut Vec<Block>) {
    blocks.push(Block::Heading {
        text: "Your hero".to_owned(),
        caption: Some(
            "A quick hero, ready to play. Tap any line to change it; the rest stays.".to_owned(),
        ),
    });
    if let Some(sheet) = &view.sheet {
        blocks.push(Block::Stats {
            stats: vec![
                stat("Hit points", sheet.hit_points.to_string(), None),
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
            ],
        });
    }
    for issue in &view.issues {
        blocks.push(notice(PillTone::Warn, issue));
    }
    let skills = view.sheet.as_ref().map_or_else(Vec::new, |sheet| {
        sheet
            .skills
            .iter()
            .filter(|skill| skill.source.is_some())
            .map(|skill| format!("{} {}", skill.name, signed(i64::from(skill.bonus))))
            .collect()
    });
    let mut rows = vec![
        review_row(
            view,
            BuildPart::Background,
            line_value(picked(view, "background")),
            "Where your hero comes from: skills, a tool, a feat and gear.",
        ),
        review_row(
            view,
            BuildPart::Abilities,
            scores_line(view.sheet.as_ref()),
            "Strength to Charisma: what every roll adds.",
        ),
        review_row(
            view,
            BuildPart::Skills,
            line_value(skills),
            "What your hero is trained in.",
        ),
        review_row(
            view,
            BuildPart::Languages,
            line_value(picked(view, "languages")),
            "Common and two more; and how your hero tends to act.",
        ),
        review_row(
            view,
            BuildPart::Equipment,
            line_value(picked(view, "class-equipment")),
            "What your hero carries into the day.",
        ),
    ];
    for (part, because) in [
        (BuildPart::Species, "The choices your people make."),
        (
            BuildPart::Class,
            "What your class asks you to choose: expertise, masteries, spells.",
        ),
    ] {
        if !view
            .groups
            .iter()
            .any(|group| BuildPart::of(&group.id) == Some(part))
        {
            continue;
        }
        rows.push(review_row(view, part, groups_line(view, part), because));
    }
    blocks.push(Block::Rows { rows });
    if let Some(sheet) = &view.sheet {
        for mark in &sheet.marks {
            blocks.push(notice(
                PillTone::Neutral,
                &format!("Not in play yet: {}", mark.note),
            ));
        }
    }
    let ready = view.state == BuildState::Reviewing && !ui.busy;
    blocks.push(Block::Buttons {
        buttons: vec![
            button("I'm ready", ButtonKind::Primary, &BuildTap::Submit, ready),
            button(
                "Surprise me",
                ButtonKind::Secondary,
                &BuildTap::Surprise,
                !ui.busy,
            ),
        ],
    });
}

fn stat(label: &str, value: String, sub: Option<String>) -> Stat {
    Stat {
        label: label.to_owned(),
        value,
        sub,
        spent: false,
    }
}

/// A part: each of its groups as cards (one pick of few, long choices) or chips, then back.
fn part_blocks(view: &CreationView, part: BuildPart, blocks: &mut Vec<Block>) {
    blocks.push(Block::Heading {
        text: part.title().to_owned(),
        caption: Some("Pick what you like; whatever you leave is chosen for you.".to_owned()),
    });
    for group in view
        .groups
        .iter()
        .filter(|group| BuildPart::of(&group.id) == Some(part))
    {
        if group.id == ABILITY_SCORES {
            score_blocks(view, group, blocks);
            continue;
        }
        if let Some(because) = &group.because {
            blocks.push(Block::Caption {
                text: format!("{}: {because}", group.label),
            });
        }
        if is_card_group(group) {
            blocks.push(Block::Cards {
                cards: group
                    .options
                    .iter()
                    .map(|option| PickCard {
                        title: option.label.clone(),
                        sub: option.not_in_play.clone(),
                        picture: None,
                        glyph: None,
                        mark: option.not_in_play.as_ref().map(|_| Mark {
                            text: "Not in play yet".to_owned(),
                            tone: PillTone::Neutral,
                        }),
                        rarity: None,
                        selected: view.picks.get(&group.slots[0]) == Some(&option.id),
                        off: false,
                        action: Some(
                            BuildTap::Pick {
                                set: vec![(group.slots[0].clone(), option.id.clone())],
                                clear: Vec::new(),
                            }
                            .action(),
                        ),
                        crop: None,
                    })
                    .collect(),
                small: false,
            });
        } else if group.options.iter().any(|option| option.detail.is_some()) {
            blocks.push(Block::Caption {
                text: pick_label(view, group),
            });
            blocks.push(rows_block(view, group));
        } else {
            blocks.push(chips_block(view, group));
        }
    }
    blocks.push(Block::Buttons {
        buttons: vec![button(
            "Back to my hero",
            ButtonKind::Secondary,
            &BuildTap::Open(None),
            true,
        )],
    });
}

/// A single pick among long choices: cards.
fn is_card_group(group: &BuildGroup) -> bool {
    group.slots.len() == 1
        && matches!(
            group.id.as_str(),
            "background"
                | "class-equipment"
                | "background-equipment"
                | "fighting-style"
                | "versatile"
        )
}

/// The tap on `option` of a group of picks, if any, and whether it is chosen. A tap on a chosen
/// option takes back exactly its slot. On another, it fills the first empty slot; a group of one
/// slot is a single choice, so its tap replaces the pick. A group of more slots that is full
/// offers no tap on an unchosen option: one must be taken back first, never replaced unseen.
fn pick_tap(view: &CreationView, group: &BuildGroup, option: &str) -> (Option<BuildTap>, bool) {
    let held = group
        .slots
        .iter()
        .find(|slot| view.picks.get(*slot).map(String::as_str) == Some(option));
    if let Some(slot) = held {
        let take_back = BuildTap::Pick {
            set: Vec::new(),
            clear: vec![slot.clone()],
        };
        return (Some(take_back), true);
    }
    let free = match group.slots.as_slice() {
        [only] => Some(only),
        slots => slots.iter().find(|slot| !view.picks.contains_key(*slot)),
    };
    let fill = free.map(|slot| BuildTap::Pick {
        set: vec![(slot.clone(), option.to_owned())],
        clear: Vec::new(),
    });
    (fill, false)
}

/// The label of a group of picks: for more than one, how many of them are chosen, and how to change
/// a full group.
fn pick_label(view: &CreationView, group: &BuildGroup) -> String {
    let count = group.slots.len();
    if count < 2 {
        return group.label.clone();
    }
    let chosen = group
        .slots
        .iter()
        .filter(|slot| view.picks.contains_key(*slot))
        .count();
    if chosen < count {
        format!("{}: {chosen} of {count} chosen", group.label)
    } else {
        format!(
            "{}: {count} of {count} chosen — take one back to change",
            group.label
        )
    }
}

/// A choice as chips.
fn chips_block(view: &CreationView, group: &BuildGroup) -> Block {
    let chips = group
        .options
        .iter()
        .map(|option| {
            let (tap, chosen) = pick_tap(view, group, &option.id);
            Chip {
                text: match &option.not_in_play {
                    Some(_) => format!("{} (not in play yet)", option.label),
                    None => option.label.clone(),
                },
                selected: chosen,
                removable: false,
                action: tap.map(|tap| tap.action()),
            }
        })
        .collect();
    Block::Chips {
        label: pick_label(view, group),
        chips,
    }
}

/// A choice whose options need more than a name (spells, weapon masteries), one to a line: the
/// name, the host's line about it and whether it is chosen.
fn rows_block(view: &CreationView, group: &BuildGroup) -> Block {
    let rows = group
        .options
        .iter()
        .map(|option| {
            let (tap, chosen) = pick_tap(view, group, &option.id);
            let mark = if chosen {
                Some(Mark {
                    text: "Chosen".to_owned(),
                    tone: PillTone::Good,
                })
            } else {
                option.not_in_play.as_ref().map(|_| Mark {
                    text: "Not in play yet".to_owned(),
                    tone: PillTone::Neutral,
                })
            };
            Row {
                label: option.label.clone(),
                value: None,
                because: option.detail.clone().or_else(|| option.not_in_play.clone()),
                change: tap
                    .as_ref()
                    .map(|_| if chosen { "Take back" } else { "Pick" }.to_owned()),
                mark,
                picture: None,
                action: tap.map(|tap| tap.action()),
            }
        })
        .collect();
    Block::Rows { rows }
}

/// The scores, one ability to a line. Under the standard array a score another ability holds is
/// swapped with it.
fn score_blocks(view: &CreationView, group: &BuildGroup, blocks: &mut Vec<Block>) {
    let swaps = view.picks.get(ABILITY_METHOD).map(String::as_str) == Some(STANDARD_ARRAY);
    for slot in &group.slots {
        let current = view.picks.get(slot);
        let chips = group
            .options
            .iter()
            .map(|option| {
                let mut set = vec![(slot.clone(), option.id.clone())];
                if swaps
                    && let Some(other) = group
                        .slots
                        .iter()
                        .find(|other| *other != slot && view.picks.get(*other) == Some(&option.id))
                    && let Some(mine) = current
                {
                    set.push((other.clone(), mine.clone()));
                }
                Chip {
                    text: option.label.clone(),
                    selected: current == Some(&option.id),
                    removable: false,
                    action: Some(
                        BuildTap::Pick {
                            set,
                            clear: Vec::new(),
                        }
                        .action(),
                    ),
                }
            })
            .collect();
        blocks.push(Block::Chips {
            label: titled(slot),
            chips,
        });
    }
}
