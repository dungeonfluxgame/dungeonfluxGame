//! The stand-in table state and its stub `step`.
//!
//! It owns an in-memory state of the size the plan expects of a table (a few hundred elements, a few
//! thousand facts), the audiences that read it, and a `step` that applies a handful of effects. The
//! state is made up: nothing here is a rule. What it keeps from the real thing is the cost shape,
//! which is a deep structure with strings, per-audience visibility that has to be filtered, and
//! bounded growth. Generation and effects are deterministic, so two runs measure the same work.

use std::collections::VecDeque;

use serde::Serialize;

/// Players at the table.
pub const SEAT_COUNT: usize = 6;
/// The six phones and the TV.
pub const AUDIENCE_COUNT: usize = SEAT_COUNT + 1;
/// The most narration lines the state keeps.
const NARRATION_LIMIT: usize = 200;
/// Effects one `step` applies: two moves, a damage, a condition toggle, a fact, a narration line
/// and a hide or reveal.
pub const EFFECTS_PER_STEP: usize = 7;

/// Who reads a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// The phone of the seat with this index.
    Seat(u8),
    Tv,
}

impl Audience {
    /// The audience with this position in a list of `AUDIENCE_COUNT`: six seats, then the TV.
    pub fn at(index: usize) -> Self {
        u8::try_from(index)
            .ok()
            .filter(|_| index < SEAT_COUNT)
            .map_or(Self::Tv, Self::Seat)
    }
}

/// Who may see an element or a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    /// Only the seat with this index.
    Seat(u8),
    /// Nobody at the table yet.
    Hidden,
}

impl Visibility {
    pub fn is_visible_to(self, audience: Audience) -> bool {
        match (self, audience) {
            (Self::Public, _) => true,
            (Self::Seat(owner), Audience::Seat(reader)) => owner == reader,
            (Self::Seat(_), Audience::Tv) | (Self::Hidden, _) => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Hero,
    Npc,
    Foe,
    Prop,
}

/// One thing on the table: a hero, a person, a foe or a prop.
#[derive(Debug, Clone, Serialize)]
pub struct Element {
    pub id: u32,
    pub name: String,
    pub kind: ElementKind,
    pub scene: u8,
    pub x: i32,
    pub y: i32,
    pub hit_points: i32,
    pub maximum_hit_points: i32,
    pub conditions: Vec<String>,
    pub visibility: Visibility,
}

/// One thing the table knows.
#[derive(Debug, Clone, Serialize)]
pub struct Fact {
    pub id: u32,
    pub visibility: Visibility,
    pub text: String,
}

/// One public line of narration.
#[derive(Debug, Clone, Serialize)]
pub struct NarrationLine {
    pub id: u32,
    pub text: String,
}

/// How big the state is and how much of it a view shows. `representative` is exploration and
/// dialogue; `combat` puts every element in the current scene, so views are several times larger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkloadProfile {
    pub name: &'static str,
    pub elements: usize,
    pub scenes: u8,
    pub facts: usize,
    pub phone_private_facts_in_view: usize,
    pub public_facts_in_view: usize,
    pub narration_in_view: usize,
}

impl WorkloadProfile {
    /// 300 elements over 5 scenes (about 60 in view), 2,500 facts, and a view of about 10 KB.
    pub const fn representative() -> Self {
        Self {
            name: "representative",
            elements: 300,
            scenes: 5,
            facts: 2_500,
            phone_private_facts_in_view: 40,
            public_facts_in_view: 40,
            narration_in_view: 30,
        }
    }

    /// The same 300 elements all in one scene, as in a crowded fight: a view several times larger.
    pub const fn combat() -> Self {
        Self {
            name: "combat",
            elements: 300,
            scenes: 1,
            facts: 2_500,
            phone_private_facts_in_view: 40,
            public_facts_in_view: 40,
            narration_in_view: 30,
        }
    }

    /// Every element in the scene and long histories in each view, a few times the bytes of
    /// `combat`, to see how the cost grows with the size of a view.
    pub const fn heavy() -> Self {
        Self {
            name: "heavy",
            elements: 300,
            scenes: 1,
            facts: 2_500,
            phone_private_facts_in_view: 400,
            public_facts_in_view: 400,
            narration_in_view: 200,
        }
    }
}

/// The state one table owns. `Clone` is the deep copy that a copy-on-write snapshot pays when a
/// reader still holds the previous one.
#[derive(Debug, Clone, Serialize)]
pub struct TableState {
    pub revision: u64,
    pub scene: u8,
    pub elements: Vec<Element>,
    pub facts: VecDeque<Fact>,
    pub narration: VecDeque<NarrationLine>,
    #[serde(skip)]
    next_id: u32,
}

const FACT_TEXTS: [&str; 10] = [
    "The innkeeper has seen the stranger before",
    "A loose stone hides a key behind the hearth",
    "The captain owes the guild a favour",
    "Footprints lead to the cellar door",
    "The well water tastes of iron",
    "Someone has been reading the ledger at night",
    "The bridge toll doubled after the flood",
    "A bell rings once at midnight with no ringer",
    "The healer will trade herbs for news",
    "The old map marks a road that no longer exists",
];

const NARRATION_TEXTS: [&str; 6] = [
    "The door groans open and cold air spills into the hall.",
    "Rain drums on the roof while the fire burns low.",
    "A shadow moves past the window and is gone.",
    "The crowd falls quiet as the stranger steps forward.",
    "Steel rings in the courtyard, close and then closer.",
    "You smell woodsmoke and something sweeter beneath it.",
];

const ELEMENT_NAMES: [(&str, ElementKind); 3] = [
    ("villager", ElementKind::Npc),
    ("raider", ElementKind::Foe),
    ("barrel", ElementKind::Prop),
];

/// A small deterministic generator (SplitMix64): the harness needs repeatable variety, not quality.
#[derive(Debug, Clone, Copy)]
pub struct Prng(u64);

impl Prng {
    pub fn seeded(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        mixed ^ (mixed >> 31)
    }

    /// A number below `bound`, or zero when `bound` is zero.
    pub fn below(&mut self, bound: usize) -> usize {
        let bound = u64::try_from(bound).unwrap_or(u64::MAX).max(1);
        usize::try_from(self.next_u64() % bound).unwrap_or(0)
    }
}

fn cycle<T: Copy>(items: &[T], index: usize) -> Option<T> {
    items.get(index % items.len().max(1)).copied()
}

fn cycle_text(items: &[&'static str], index: usize) -> &'static str {
    cycle(items, index).unwrap_or("")
}

fn visibility_by_roll(
    roll: usize,
    seat_roll: usize,
    share_public: usize,
    share_seat: usize,
) -> Visibility {
    if roll < share_public {
        Visibility::Public
    } else if roll < share_public + share_seat {
        Visibility::Seat(u8::try_from(seat_roll % SEAT_COUNT).unwrap_or(0))
    } else {
        Visibility::Hidden
    }
}

impl TableState {
    /// A state sized by `profile`: the first six elements are the heroes, the rest are spread over
    /// the scenes, and about one fact in three is private to a seat.
    pub fn generate(profile: &WorkloadProfile) -> Self {
        let mut prng = Prng::seeded(0x00d1_5ea5);
        let mut next_id = 0u32;
        let mut take_id = || {
            next_id += 1;
            next_id
        };
        let elements = (0..profile.elements)
            .map(|index| {
                let id = take_id();
                let hero = index < SEAT_COUNT;
                let (stem, kind) = if hero {
                    ("hero", ElementKind::Hero)
                } else {
                    cycle(&ELEMENT_NAMES, index).unwrap_or(("prop", ElementKind::Prop))
                };
                let hit_points = 8 + i32::try_from(prng.below(40)).unwrap_or(8);
                Element {
                    id,
                    name: format!("{stem}-{index:03}"),
                    kind,
                    scene: if hero {
                        0
                    } else {
                        u8::try_from(index % usize::from(profile.scenes.max(1))).unwrap_or(0)
                    },
                    x: i32::try_from(prng.below(2_000)).unwrap_or(0),
                    y: i32::try_from(prng.below(2_000)).unwrap_or(0),
                    hit_points,
                    maximum_hit_points: hit_points + 6,
                    conditions: if index % 7 == 0 {
                        vec!["prone".to_owned()]
                    } else {
                        Vec::new()
                    },
                    visibility: if hero {
                        Visibility::Public
                    } else {
                        visibility_by_roll(prng.below(100), index, 85, 10)
                    },
                }
            })
            .collect();
        let facts = (0..profile.facts)
            .map(|index| {
                let id = take_id();
                Fact {
                    id,
                    visibility: visibility_by_roll(prng.below(100), prng.below(SEAT_COUNT), 60, 30),
                    text: format!("{} ({index})", cycle_text(&FACT_TEXTS, index)),
                }
            })
            .collect();
        let narration = (0..NARRATION_LIMIT)
            .map(|index| NarrationLine {
                id: take_id(),
                text: cycle_text(&NARRATION_TEXTS, index).to_owned(),
            })
            .collect();
        Self {
            revision: 1,
            scene: 0,
            elements,
            facts,
            narration,
            next_id,
        }
    }

    fn take_id(&mut self) -> u32 {
        self.next_id = self.next_id.wrapping_add(1);
        self.next_id
    }

    /// The stub `step`: applies `EFFECTS_PER_STEP` effects for one served input. `turn` seeds the
    /// choices, so a replay of the same turns gives the same state. Growth is bounded: a new fact
    /// evicts the oldest past `profile.facts`, and narration is capped.
    pub fn step(&mut self, profile: &WorkloadProfile, turn: u64) {
        let mut prng = Prng::seeded(turn);
        for _ in 0..2 {
            self.move_element(&mut prng);
        }
        self.damage_element(&mut prng);
        self.toggle_condition(&mut prng);
        self.add_fact(profile, &mut prng, turn);
        self.add_narration(&mut prng);
        self.toggle_hidden(&mut prng);
        self.revision = self.revision.saturating_add(1);
    }

    fn move_element(&mut self, prng: &mut Prng) {
        let index = prng.below(self.elements.len());
        let dx = i32::try_from(prng.below(7)).unwrap_or(3) - 3;
        let dy = i32::try_from(prng.below(7)).unwrap_or(3) - 3;
        if let Some(element) = self.elements.get_mut(index) {
            element.x = element.x.saturating_add(dx * 5);
            element.y = element.y.saturating_add(dy * 5);
        }
    }

    fn damage_element(&mut self, prng: &mut Prng) {
        let index = prng.below(self.elements.len());
        let amount = i32::try_from(prng.below(9)).unwrap_or(0) - 3;
        if let Some(element) = self.elements.get_mut(index) {
            element.hit_points = element
                .hit_points
                .saturating_sub(amount)
                .clamp(0, element.maximum_hit_points);
        }
    }

    fn toggle_condition(&mut self, prng: &mut Prng) {
        let index = prng.below(self.elements.len());
        if let Some(element) = self.elements.get_mut(index)
            && element.conditions.pop().is_none()
        {
            element.conditions.push("prone".to_owned());
        }
    }

    fn add_fact(&mut self, profile: &WorkloadProfile, prng: &mut Prng, turn: u64) {
        let id = self.take_id();
        let text = format!(
            "{} (turn {turn})",
            cycle_text(&FACT_TEXTS, prng.below(FACT_TEXTS.len()))
        );
        let visibility = visibility_by_roll(prng.below(100), prng.below(SEAT_COUNT), 60, 30);
        self.facts.push_back(Fact {
            id,
            visibility,
            text,
        });
        while self.facts.len() > profile.facts {
            self.facts.pop_front();
        }
    }

    fn add_narration(&mut self, prng: &mut Prng) {
        let id = self.take_id();
        let text = cycle_text(&NARRATION_TEXTS, prng.below(NARRATION_TEXTS.len())).to_owned();
        self.narration.push_back(NarrationLine { id, text });
        while self.narration.len() > NARRATION_LIMIT {
            self.narration.pop_front();
        }
    }

    fn toggle_hidden(&mut self, prng: &mut Prng) {
        let index = SEAT_COUNT + prng.below(self.elements.len().saturating_sub(SEAT_COUNT));
        if let Some(element) = self.elements.get_mut(index) {
            element.visibility = match element.visibility {
                Visibility::Hidden => Visibility::Public,
                Visibility::Public | Visibility::Seat(_) => Visibility::Hidden,
            };
        }
    }
}
