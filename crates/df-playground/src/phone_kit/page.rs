//! The blocks the phone's pages are made of: creation, the bag, clues, the map, the hero sheet, the
//! notebook and battle mode. A page is a column of blocks inside the one scrolling body, so each
//! block is a small typed input (sample data today) and the decisions its drawing needs are the
//! functions here, which native tests cover. A block decides no rule: every number, mark and
//! reason is what the server would send.

use serde::{Deserialize, Serialize};

use crate::kit::Rarity;
use crate::overlay::Glyph;
use crate::phone_kit::shell::{ButtonSpec, PillTone};
use crate::speaker_card::PortraitCrop;

/// A small word on a card or a row: a rarity, "Not in play yet", "Seen".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub text: String,
    #[serde(default)]
    pub tone: PillTone,
}

/// An illustrated card to pick or to read: a species, an item, a clue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PickCard {
    pub title: String,
    #[serde(default)]
    pub sub: Option<String>,
    /// A picture of the media library; a glyph, then the first letter, stand in without one.
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default)]
    pub glyph: Option<Glyph>,
    #[serde(default)]
    pub mark: Option<Mark>,
    /// An item's rarity, which tints the card's rim.
    #[serde(default)]
    pub rarity: Option<Rarity>,
    #[serde(default)]
    pub selected: bool,
    /// Greyed out, with the reason in `sub`.
    #[serde(default)]
    pub off: bool,
    /// What a tap asks for (`nav::page_action`), when the card goes somewhere.
    #[serde(default)]
    pub action: Option<String>,
    /// The cut of `picture` the card shows, when it is a face cut from a room picture
    /// (`speaker_card::portrait_crop`). Never read from a sample file.
    #[serde(skip)]
    pub crop: Option<PortraitCrop>,
}

/// One line of a list: a label with its value, the reason for it and a way to change it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub label: String,
    #[serde(default)]
    pub value: Option<String>,
    /// The "because" under the line.
    #[serde(default)]
    pub because: Option<String>,
    /// The word of the button that changes it.
    #[serde(default)]
    pub change: Option<String>,
    #[serde(default)]
    pub mark: Option<Mark>,
    #[serde(default)]
    pub picture: Option<String>,
    /// What a tap on the change button asks for.
    #[serde(default)]
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chip {
    pub text: String,
    #[serde(default)]
    pub selected: bool,
    /// Drawn with a remove mark, for a detail the player may take out.
    #[serde(default)]
    pub removable: bool,
    #[serde(default)]
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Swatch {
    pub name: String,
    /// A plain hex colour.
    pub color: String,
    #[serde(default)]
    pub selected: bool,
    /// What a tap asks for, when the swatch can be picked.
    #[serde(default)]
    pub action: Option<String>,
}

/// A slider the player moves: its whole-number range and value, and the action its change sends
/// with the new value after a colon (`look:slider:build` sends `look:slider:build:3`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliderRange {
    pub action: String,
    pub min: u32,
    pub max: u32,
    pub value: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stat {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub sub: Option<String>,
    /// A spent or unavailable stat is drawn dim.
    #[serde(default)]
    pub spent: bool,
}

/// What the look studio's stage shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageState {
    #[default]
    Ready,
    /// A picture is being made: a loader turns over the silhouette.
    Generating,
    /// The picture could not be made: the kept one stays.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StripItem {
    pub label: String,
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default)]
    pub selected: bool,
    /// What a tap asks for, when the item can be gone back to.
    #[serde(default)]
    pub action: Option<String>,
}

/// How far the player has come with a place on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    /// The party is here.
    Here,
    #[default]
    Visited,
    /// Named to the party or seen on the way, not visited.
    Unvisited,
}

impl NodeState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Here => "here",
            Self::Visited => "visited",
            Self::Unvisited => "unvisited",
        }
    }
}

/// A place on the map, by the name the party knows. `x` and `y` are shares of the map's size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapNode {
    pub name: String,
    pub x: u8,
    pub y: u8,
    #[serde(default)]
    pub plate: Option<String>,
    #[serde(default)]
    pub state: NodeState,
    #[serde(default)]
    pub action: Option<String>,
}

/// A way between two places, named by their names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapWay {
    pub from: String,
    pub to: String,
    /// A way that cannot be taken now.
    #[serde(default)]
    pub blocked: bool,
}

impl MapWay {
    /// The two ends of the way as shares of the map, or `None` when a name is not a place.
    pub fn ends(&self, nodes: &[MapNode]) -> Option<([u8; 2], [u8; 2])> {
        let find = |name: &str| nodes.iter().find(|node| node.name == name);
        let (from, to) = (find(&self.from)?, find(&self.to)?);
        Some(([from.x, from.y], [to.x, to.y]))
    }
}

/// What one square of a fight is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Square {
    Floor,
    Wall,
    Cover,
    Difficult,
}

impl Square {
    /// The square a character of a `Grid`'s terrain rows stands for: `.` floor, `#` wall, `c`
    /// cover, `~` difficult ground.
    pub fn of(code: char) -> Option<Self> {
        match code {
            '.' => Some(Self::Floor),
            '#' => Some(Self::Wall),
            'c' => Some(Self::Cover),
            '~' => Some(Self::Difficult),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Floor => "floor",
            Self::Wall => "wall",
            Self::Cover => "cover",
            Self::Difficult => "difficult",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Me,
    Ally,
    Foe,
}

impl Side {
    pub fn key(self) -> &'static str {
        match self {
            Self::Me => "me",
            Self::Ally => "ally",
            Self::Foe => "foe",
        }
    }
}

/// How hurt a creature looks, in words and never in numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wound {
    Unhurt,
    Bloodied,
    NearDeath,
}

impl Wound {
    pub fn word(self) -> &'static str {
        match self {
            Self::Unhurt => "Unhurt",
            Self::Bloodied => "Bloodied",
            Self::NearDeath => "Near death",
        }
    }

    pub fn tone(self) -> PillTone {
        match self {
            Self::Unhurt => PillTone::Good,
            Self::Bloodied => PillTone::Warn,
            Self::NearDeath => PillTone::Danger,
        }
    }
}

/// A figure on the grid, by the name the hero knows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Token {
    /// Column and row, from 0.
    pub at: [u8; 2],
    pub name: String,
    #[serde(default)]
    pub portrait: Option<String>,
    pub side: Side,
    #[serde(default)]
    pub wound: Option<Wound>,
}

/// The fight's map as the combat machine would send it: the terrain, the squares the hero can
/// reach, the path being planned, the figures and the target.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Grid {
    /// One string per row; see `Square::of`.
    pub terrain: Vec<String>,
    #[serde(default)]
    pub reach: Vec<[u8; 2]>,
    #[serde(default)]
    pub path: Vec<[u8; 2]>,
    #[serde(default)]
    pub tokens: Vec<Token>,
    #[serde(default)]
    pub target: Option<[u8; 2]>,
}

impl Grid {
    /// Columns and rows.
    pub fn size(&self) -> (usize, usize) {
        (
            self.terrain.first().map_or(0, |row| row.chars().count()),
            self.terrain.len(),
        )
    }

    /// The square at a column and row; an unknown code or a place off the grid is a floor.
    pub fn square(&self, column: usize, row: usize) -> Square {
        self.terrain
            .get(row)
            .and_then(|line| line.chars().nth(column))
            .and_then(Square::of)
            .unwrap_or(Square::Floor)
    }

    fn at(point: &[u8; 2], column: usize, row: usize) -> bool {
        usize::from(point[0]) == column && usize::from(point[1]) == row
    }

    pub fn is_lit(&self, column: usize, row: usize) -> bool {
        self.reach.iter().any(|point| Self::at(point, column, row))
    }

    /// Which step of the path this square is (from 1), when it is on the path.
    pub fn path_step(&self, column: usize, row: usize) -> Option<usize> {
        self.path
            .iter()
            .position(|point| Self::at(point, column, row))
            .map(|index| index + 1)
    }

    pub fn is_target(&self, column: usize, row: usize) -> bool {
        self.target
            .is_some_and(|point| Self::at(&point, column, row))
    }

    pub fn token_at(&self, column: usize, row: usize) -> Option<&Token> {
        self.tokens
            .iter()
            .find(|token| Self::at(&token.at, column, row))
    }

    /// What is wrong with the grid, if anything: rows of unequal length or a code that is none.
    pub fn problem(&self) -> Option<String> {
        let (columns, _) = self.size();
        for (index, row) in self.terrain.iter().enumerate() {
            if row.chars().count() != columns {
                return Some(format!("row {} is not {columns} squares long", index + 1));
            }
            if let Some(code) = row.chars().find(|code| Square::of(*code).is_none()) {
                return Some(format!("'{code}' is not a square"));
            }
        }
        None
    }
}

/// One block of a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    /// A title with an italic line under it.
    Heading {
        text: String,
        #[serde(default)]
        caption: Option<String>,
    },
    /// A caption between gold rules.
    Caption {
        text: String,
    },
    Paragraph {
        text: String,
    },
    /// Illustrated cards in a wrapping grid; `small` makes three to a row.
    Cards {
        cards: Vec<PickCard>,
        #[serde(default)]
        small: bool,
    },
    Rows {
        rows: Vec<Row>,
    },
    /// A label over a wrapping row of chips.
    Chips {
        label: String,
        chips: Vec<Chip>,
    },
    Slider {
        label: String,
        percent: u8,
        low: String,
        high: String,
        /// The input the player moves; a slider without one only shows its value.
        #[serde(default)]
        range: Option<SliderRange>,
    },
    Swatches {
        label: String,
        swatches: Vec<Swatch>,
    },
    /// A text field with the rule it is held to shown under it.
    Field {
        label: String,
        value: String,
        placeholder: String,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        tall: bool,
        /// The element id the page reads the typed value from.
        #[serde(default)]
        id: Option<String>,
    },
    Stats {
        stats: Vec<Stat>,
    },
    /// A bar with a label, such as the time left to answer.
    Meter {
        label: String,
        percent: u8,
        #[serde(default)]
        tone: PillTone,
    },
    /// The look studio's stage: a large figure, with a loader over the
    /// silhouette while one is made.
    Stage {
        picture: Option<String>,
        #[serde(default)]
        state: StageState,
        caption: String,
        /// What a tap on the picture asks for (opening it full screen).
        #[serde(default)]
        action: Option<String>,
    },
    /// The new picture beside the kept one; each side may be tapped (`actions`: kept, new).
    Compare {
        kept: String,
        fresh: String,
        caption: String,
        #[serde(default)]
        actions: Option<[String; 2]>,
    },
    /// Pictures in a wrapping row: the versions of a look, the turn order of a fight.
    Strip {
        label: String,
        items: Vec<StripItem>,
    },
    Buttons {
        buttons: Vec<ButtonSpec>,
    },
    /// A line of warning or advice with a glyph.
    Notice {
        tone: PillTone,
        text: String,
    },
    Map {
        nodes: Vec<MapNode>,
        ways: Vec<MapWay>,
    },
    Grid(Grid),
    /// An item looked at closely, as render 01: the picture large with its title block.
    Inspect {
        picture: String,
        title: String,
        #[serde(default)]
        rarity: Option<Rarity>,
        text: String,
    },
    /// Death saves as a tally.
    Tally {
        label: String,
        successes: u8,
        failures: u8,
    },
}

/// How many marks of a death-save tally are drawn: the rules' three.
pub const TALLY_MARKS: u8 = 3;

/// Share of a bar, held to a whole.
pub fn bar_share(percent: u8) -> u8 {
    percent.min(100)
}

/// The words of a death-save tally: "2 successes, 1 failure".
pub fn tally_text(successes: u8, failures: u8) -> String {
    let plural = |count: u8, one: &str, many: &str| {
        let word = if count == 1 { one } else { many };
        format!("{count} {word}")
    };
    format!(
        "{}, {}",
        plural(successes.min(TALLY_MARKS), "success", "successes"),
        plural(failures.min(TALLY_MARKS), "failure", "failures")
    )
}
