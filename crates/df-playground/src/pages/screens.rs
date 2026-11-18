//! The live pages as the phone kit's panels: what each shows and what each tap asks for, decided
//! here from the host's `SeatPages`, the seat's story and what the table asks the seat now.
//!
//! A tap carries a `PageTap` as its `data-action`; moving between pages is the kit's navigation
//! (`phone_kit::nav::page_action`). A page that is not one of these is left to the kit's samples.

use crate::clues::ClueBoard;
use crate::lobby::model::BubbleTone;
use crate::look::studio::{LookViewer, ViewItem};
use crate::phone_kit::PhonePanels;
use crate::phone_kit::nav::page_action;
use crate::phone_kit::page::{Block, Chip, MapNode, MapWay, Mark, NodeState, PickCard, Row, Stat};
use crate::phone_kit::shell::{ButtonKind, ButtonSpec, EmptyState, PillTone, TabBadge, TabKey};
use crate::phone_kit::talk::{TalkPanel, ThreadLine};
use crate::seat::wire::{SeatView, StoryLine};
use crate::speaker_card::Portrait;

use super::wire::{BagItem, BagLoad, Kept, KeptKind, Person, SeatPages};

/// The page of what the character carries.
pub const BAG_PAGE: &str = "bag";
pub const TALK_PAGE: &str = "talk";
pub const NOTES_PAGE: &str = "notes";
pub const MAP_PAGE: &str = "map";
/// The clue board, as the shared screen shows it (`app::pages::clues_board`).
pub const CLUES_PAGE: &str = "clues";
pub const LOOK_PAGE: &str = "look";
/// Every painting the character made.
pub const GALLERY_PAGE: &str = "gallery";
/// The page of one item: `item-<id>`.
const ITEM_PREFIX: &str = "item-";
/// The page of one painting: `painting-<id>`.
const PAINTING_PREFIX: &str = "painting-";
/// The page of one shop here: `shop-<id>`, its keeper's id.
const SHOP_PREFIX: &str = "shop-";

/// The element id of the Talk page's words.
pub const TALK_FIELD_ID: &str = "pages-talk";
/// The element id of the Notebook's note.
pub const NOTE_FIELD_ID: &str = "pages-note";

/// The page of the item `id`.
pub fn item_page(id: &str) -> String {
    format!("{ITEM_PREFIX}{id}")
}

/// The page of the painting `id`.
pub fn painting_page(id: &str) -> String {
    format!("{PAINTING_PREFIX}{id}")
}

/// The page of the shop `id`.
pub fn shop_page(id: &str) -> String {
    format!("{SHOP_PREFIX}{id}")
}

/// What the table waits on from this seat now: its open question (the prompt) and the roll it
/// called (its title). The phone marks the tab each waits on and puts a line on every other page,
/// and never moves the player off the page they are reading.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableWaits {
    pub ask: Option<String>,
    pub roll: Option<String>,
}

/// The line on a page that says the table waits, and the tap that goes to where it is answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitBanner {
    pub text: String,
    pub action: String,
}

impl TableWaits {
    /// What the seat's view says the table waits on.
    pub fn of(view: &SeatView) -> Self {
        Self {
            ask: view.ask.as_ref().map(|ask| ask.prompt.clone()),
            roll: view.pending_roll.as_ref().map(|roll| roll.title.clone()),
        }
    }

    /// A pulsing dot on Act while a question is open, and on Roll while a roll is called.
    pub fn badges(&self) -> Vec<TabBadge> {
        [
            (TabKey::Act, self.ask.is_some()),
            (TabKey::Roll, self.roll.is_some()),
        ]
        .into_iter()
        .filter(|(_, waits)| *waits)
        .map(|(tab, _)| TabBadge {
            tab,
            count: 0,
            waiting: true,
        })
        .collect()
    }

    /// The line on `page`: the open question on any page but Act, else the called roll on any
    /// page but Act and Roll (both show it there).
    pub fn banner(&self, page: &str) -> Option<WaitBanner> {
        let act = TabKey::Act.page_key();
        if let Some(prompt) = self.ask.as_ref().filter(|_| page != act) {
            return Some(WaitBanner {
                text: format!("The table asks: {prompt} — tap to answer"),
                action: page_action(act),
            });
        }
        let roll = super::sheet::ROLL_PAGE;
        self.roll
            .as_ref()
            .filter(|_| page != act && page != roll)
            .map(|title| WaitBanner {
                text: format!("The table calls for {title} — tap to roll"),
                action: page_action(roll),
            })
    }
}

/// The panel of the tab bar a live page belongs to; none for a page that is not one of these (the
/// paintings and close looks, reached from "more", which then lights).
pub fn tab_of(page: &str) -> Option<TabKey> {
    match page {
        BAG_PAGE => Some(TabKey::Inventory),
        TALK_PAGE => Some(TabKey::Talk),
        NOTES_PAGE => Some(TabKey::Notebook),
        MAP_PAGE => Some(TabKey::Map),
        CLUES_PAGE => Some(TabKey::Clues),
        _ if page.starts_with(ITEM_PREFIX) => Some(TabKey::Inventory),
        // A shop is reached from Act.
        _ if page.starts_with(SHOP_PREFIX) => Some(TabKey::Act),
        _ => None,
    }
}

/// What the Notebook shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotesFilter {
    #[default]
    All,
    Kind(KeptKind),
    /// The lines heard so far, each to keep.
    Recent,
}

impl NotesFilter {
    const CHOICES: [Self; 6] = [
        Self::All,
        Self::Kind(KeptKind::Place),
        Self::Kind(KeptKind::Painting),
        Self::Kind(KeptKind::Snap),
        Self::Kind(KeptKind::Note),
        Self::Recent,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Kind(kind) => kind.label(),
            Self::Recent => "Recent lines",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Kind(kind) => kind.key(),
            Self::Recent => "recent",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::CHOICES.into_iter().find(|filter| filter.key() == key)
    }
}

/// A tap on a live page, carried by its element as `data-action`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageTap {
    /// Say the line of suggested card `index` of what the table asks now.
    Say(usize),
    /// Speak to the person `id`, or to the room.
    Address(Option<String>),
    /// Send the Talk page's words.
    Send,
    Filter(NotesFilter),
    /// Keep the Notebook's note.
    Write,
    /// Snap the main screen.
    Snap,
    /// Keep the story's line of the id `line`.
    Keep(u64),
    /// A place on the map that cannot be gone to now: `id`.
    Refused(String),
    /// Trade with the keeper `id`: Talk opens with them addressed and a line ready.
    Trade(String),
    /// Use one of the item `id`.
    Use(String),
    /// Drop one of the item `id`.
    Drop(String),
    /// Offer one of the item `id` for nothing to the one the player speaks to.
    Give(String),
    /// Offer one of the item `id` for its price to the one the player speaks to.
    Sell(String),
    /// Start or stop a recording of the player's words for the Talk field.
    Mic,
    /// Turn the seat's talking by voice on or off.
    Speaking(bool),
    /// Open the clue `id` full screen on this phone.
    Clue(String),
    /// Close the clue open full screen on this phone; the table is not told.
    CloseClue,
    /// Show the clue `id` full screen on the shared screen, in place of any shown.
    ShowClue(String),
    /// Take the clue the shared screen shows off it, whoever put it up.
    TakeClueOff,
    /// Buy `qty` of the item `item` at the shop `shop` (its keeper's id).
    Buy {
        shop: String,
        item: String,
        qty: u32,
    },
    /// Cast the spell `spell` (its id) at `level` (0 for a cantrip), as a ritual when `ritual`.
    Cast {
        spell: String,
        level: u8,
        ritual: bool,
    },
}

const TAP_PREFIX: &str = "pages:";

impl PageTap {
    pub fn action(&self) -> String {
        let word = match self {
            Self::Say(index) => format!("say-{index}"),
            Self::Address(None) => "to-room".to_owned(),
            Self::Address(Some(id)) => format!("to-{id}"),
            Self::Send => "send".to_owned(),
            Self::Filter(filter) => format!("filter-{}", filter.key()),
            Self::Write => "write".to_owned(),
            Self::Snap => "snap".to_owned(),
            Self::Keep(line) => format!("keep-{line}"),
            Self::Refused(id) => format!("refused-{id}"),
            Self::Trade(id) => format!("trade-{id}"),
            Self::Use(id) => format!("use-{id}"),
            Self::Drop(id) => format!("drop-{id}"),
            Self::Give(id) => format!("give-{id}"),
            Self::Sell(id) => format!("sell-{id}"),
            Self::Mic => "mic".to_owned(),
            Self::Speaking(true) => "speaking-on".to_owned(),
            Self::Speaking(false) => "speaking-off".to_owned(),
            Self::Clue(id) => format!("clue-{id}"),
            Self::CloseClue => "close-clue".to_owned(),
            Self::ShowClue(id) => format!("show-clue-{id}"),
            Self::TakeClueOff => "take-clue-off".to_owned(),
            // Ids are lowercase letters, digits, `_` and `-`: the dot cannot be in one.
            Self::Buy { shop, item, qty } => format!("buy-{qty}-{shop}.{item}"),
            Self::Cast {
                spell,
                level,
                ritual,
            } => format!(
                "cast-{}{level}-{spell}",
                if *ritual { "ritual-" } else { "" }
            ),
        };
        format!("{TAP_PREFIX}{word}")
    }

    /// The tap an action names, or `None` for another control's.
    pub fn from_action(action: &str) -> Option<Self> {
        let word = action.strip_prefix(TAP_PREFIX)?;
        let index = |prefix: &str| word.strip_prefix(prefix)?.parse::<usize>().ok();
        let id = |prefix: &str| {
            word.strip_prefix(prefix)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
        };
        match word {
            "to-room" => Some(Self::Address(None)),
            "send" => Some(Self::Send),
            "write" => Some(Self::Write),
            "snap" => Some(Self::Snap),
            "mic" => Some(Self::Mic),
            "speaking-on" => Some(Self::Speaking(true)),
            "speaking-off" => Some(Self::Speaking(false)),
            "close-clue" => Some(Self::CloseClue),
            "take-clue-off" => Some(Self::TakeClueOff),
            _ => index("say-")
                .map(Self::Say)
                .or_else(|| {
                    word.strip_prefix("keep-")
                        .and_then(|line| line.parse::<u64>().ok())
                        .map(Self::Keep)
                })
                .or_else(|| {
                    word.strip_prefix("filter-")
                        .and_then(NotesFilter::from_key)
                        .map(Self::Filter)
                })
                .or_else(|| id("to-").map(|id| Self::Address(Some(id))))
                .or_else(|| id("refused-").map(Self::Refused))
                .or_else(|| id("trade-").map(Self::Trade))
                .or_else(|| id("use-").map(Self::Use))
                .or_else(|| id("drop-").map(Self::Drop))
                .or_else(|| id("give-").map(Self::Give))
                .or_else(|| id("sell-").map(Self::Sell))
                .or_else(|| id("show-clue-").map(Self::ShowClue))
                .or_else(|| id("clue-").map(Self::Clue))
                .or_else(|| {
                    let (qty, ware) = word.strip_prefix("buy-")?.split_once('-')?;
                    let (shop, item) = ware.split_once('.')?;
                    let named = |id: &str| Some(id).filter(|id| !id.is_empty()).map(str::to_owned);
                    Some(Self::Buy {
                        shop: named(shop)?,
                        item: named(item)?,
                        qty: qty.parse().ok()?,
                    })
                })
                .or_else(|| {
                    let cast = word.strip_prefix("cast-")?;
                    let (ritual, cast) = match cast.strip_prefix("ritual-") {
                        Some(rest) => (true, rest),
                        None => (false, cast),
                    };
                    let (level, spell) = cast.split_once('-')?;
                    Some(Self::Cast {
                        spell: Some(spell).filter(|spell| !spell.is_empty())?.to_owned(),
                        level: level.parse().ok()?,
                        ritual,
                    })
                }),
        }
    }
}

/// What the phone keeps of its pages: whom the hero speaks to, the drafts, the Notebook's filter,
/// and the last refusal or problem to show.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PagesUi {
    /// The person addressed; none is the room.
    pub addressing: Option<String>,
    pub talk_draft: String,
    pub note_draft: String,
    pub filter: NotesFilter,
    /// Why the last map tap could not be gone to.
    pub refused: Option<String>,
    /// The host's sentence for the last thing it would not take.
    pub problem: Option<String>,
    /// The last roll asked for from the Roll page.
    pub roll: Option<super::sheet::RollShown>,
    /// The clue open full screen on this phone, by id.
    pub clue: Option<String>,
}

/// A suggested card of what the table asks now: its index, and what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WayCard {
    pub index: usize,
    pub label: String,
}

/// A suggested card that steps up to the thing `thing` (its id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookCard {
    pub index: usize,
    pub thing: String,
}

/// The Talk page's microphone: whether the table writes speech down, the seat's switch, and the
/// one recorder the phone has (shared with the Act page's composer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TalkMic {
    /// The table has no speech to text: no microphone shows.
    #[default]
    Unavailable,
    /// Talking by voice is turned off for this seat.
    Off,
    Ready,
    /// Recording for the Talk field.
    Recording,
    /// The recorder is asking for the microphone, writing a recording down, or in use on Act.
    Busy,
}

/// Everything the live pages are drawn from.
#[derive(Debug, Clone, Copy)]
pub struct PagesInput<'a> {
    pub pages: &'a SeatPages,
    /// The character's name.
    pub hero: &'a str,
    /// The lines the table heard, oldest first.
    pub lines: &'a [StoryLine],
    /// The faces of people here cut from the shared screen's room picture, by persona id.
    pub faces: &'a [(String, Portrait)],
    /// The ways to go, when the table asks where the hero goes next.
    pub ways: &'a [WayCard],
    /// The things to step up to, when the table asks what the hero looks at.
    pub looks: &'a [LookCard],
    /// The Talk lines sent that the table has not taken yet.
    pub talk_waiting: u32,
    pub mic: TalkMic,
    /// The roll the dungeon master calls for from this seat, if any.
    pub called: Option<&'a crate::seat::wire::PendingRoll>,
    /// The clue board the shared screen shows, and the clue it shows full screen, by id.
    pub clues: Option<&'a ClueBoard>,
    pub on_table: Option<&'a str>,
    pub ui: &'a PagesUi,
}

/// The panels of the live page `page`, or `None` when it is not one of these.
pub fn panels(page: &str, input: &PagesInput<'_>) -> Option<PhonePanels> {
    let panels = match page {
        BAG_PAGE => bag(input),
        TALK_PAGE => talk(input),
        NOTES_PAGE => notes(input),
        MAP_PAGE => map(input),
        CLUES_PAGE => clues(input),
        LOOK_PAGE => look(input),
        GALLERY_PAGE => gallery(input),
        super::sheet::HERO_PAGE => {
            super::sheet::hero_page(input.pages, input.hero, input.ui.problem.as_deref())
        }
        super::sheet::ROLL_PAGE => {
            super::sheet::roll_page(input.pages, input.ui.roll.as_ref(), input.called)
        }
        _ => {
            if let Some(id) = page.strip_prefix(ITEM_PREFIX) {
                item(input, id)
            } else if let Some(id) = page.strip_prefix(SHOP_PREFIX) {
                shop(input, id)
            } else {
                painting(input, page.strip_prefix(PAINTING_PREFIX)?)
            }
        }
    };
    Some(panels)
}

fn blocks(blocks: Vec<Block>) -> PhonePanels {
    PhonePanels {
        blocks,
        ..PhonePanels::default()
    }
}

fn empty(title: &str, text: &str) -> EmptyState {
    EmptyState {
        title: title.to_owned(),
        text: text.to_owned(),
        glyph: None,
    }
}

fn heading(text: &str, caption: Option<String>) -> Block {
    Block::Heading {
        text: text.to_owned(),
        caption,
    }
}

fn notice(tone: PillTone, text: &str) -> Block {
    Block::Notice {
        tone,
        text: text.to_owned(),
    }
}

fn button(label: &str, kind: ButtonKind, action: String) -> ButtonSpec {
    ButtonSpec {
        label: label.to_owned(),
        kind,
        enabled: true,
        reason: None,
        action: Some(action),
    }
}

fn back_button(label: &str, page: &str) -> Block {
    Block::Buttons {
        buttons: vec![button(label, ButtonKind::Secondary, page_action(page))],
    }
}

fn problem(input: &PagesInput<'_>) -> Option<Block> {
    input
        .ui
        .problem
        .as_deref()
        .map(|text| notice(PillTone::Warn, text))
}

fn card(title: &str, sub: Option<String>, picture: Option<&String>) -> PickCard {
    PickCard {
        title: title.to_owned(),
        sub,
        picture: picture.cloned(),
        glyph: None,
        mark: None,
        rarity: None,
        selected: false,
        off: false,
        action: None,
        crop: None,
    }
}

fn count_text(count: u64) -> String {
    if count == 1 {
        "One".to_owned()
    } else {
        format!("{count} of them")
    }
}

/// What the hero carries against their carrying capacity, as a line of numbers.
fn load_stats(load: &BagLoad) -> Block {
    let carried = match &load.capacity {
        Some(capacity) => format!("{} of {capacity}", load.weight),
        None => load.weight.clone(),
    };
    Block::Stats {
        stats: vec![Stat {
            label: "You carry".to_owned(),
            value: carried,
            sub: Some(
                if load.capacity.is_some() {
                    "carrying capacity"
                } else {
                    "no sheet to weigh it by yet"
                }
                .to_owned(),
            ),
            spent: load.over,
        }],
    }
}

/// The load: the total against the capacity, a warning past it, each container against its room,
/// and what has no weight.
fn load_blocks(load: &BagLoad) -> Vec<Block> {
    let mut blocks = vec![load_stats(load)];
    if load.over {
        blocks.push(notice(
            PillTone::Warn,
            "You carry more than your carrying capacity.",
        ));
    }
    if !load.containers.is_empty() {
        blocks.push(Block::Caption {
            text: "Your bags".to_owned(),
        });
        blocks.push(Block::Rows {
            rows: load
                .containers
                .iter()
                .map(|container| Row {
                    label: if container.count > 1 {
                        format!("{} ({})", container.name, container.count)
                    } else {
                        container.name.clone()
                    },
                    value: Some(match &container.capacity {
                        Some(capacity) => format!("{} of {capacity}", container.load),
                        None => container.load.clone(),
                    }),
                    because: container.shares.clone(),
                    change: None,
                    mark: container.full.then(|| Mark {
                        text: "Full".to_owned(),
                        tone: PillTone::Warn,
                    }),
                    picture: None,
                    action: None,
                })
                .collect(),
        });
    }
    if !load.unweighed.is_empty() {
        blocks.push(Block::Paragraph {
            text: format!("Not weighed: {}.", load.unweighed.join(", ")),
        });
    }
    blocks
}

/// A carried thing's line under its name: how many, what they weigh and where they are packed.
fn carried_text(item: &BagItem) -> String {
    [
        Some(count_text(item.count)),
        item.weight.clone(),
        item.stowed.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

fn bag(input: &PagesInput<'_>) -> PhonePanels {
    let pages = input.pages;
    let mut page = vec![heading("What you carry", pages.place.clone())];
    let cards = pages
        .items
        .iter()
        .map(|item| PickCard {
            action: Some(page_action(&item_page(&item.id))),
            ..card(&item.name, Some(carried_text(item)), item.picture.as_ref())
        })
        .collect::<Vec<_>>();
    let carries_nothing = cards.is_empty();
    if let Some(purse) = &pages.purse {
        page.push(notice(PillTone::Neutral, &format!("Your purse: {purse}")));
    }
    if let Some(load) = &pages.load {
        page.extend(load_blocks(load));
    }
    if !carries_nothing {
        page.push(Block::Cards { cards, small: true });
    }
    if !pages.keepers.is_empty() {
        page.push(heading(
            "Trade here",
            Some("Say what you want: the keeper answers and your bag follows.".to_owned()),
        ));
        page.push(Block::Cards {
            cards: pages
                .keepers
                .iter()
                .map(|keeper| PickCard {
                    action: Some(PageTap::Trade(keeper.id.clone()).action()),
                    ..card(
                        &format!("Buy or trade with {}", keeper.name),
                        keeper.about.clone(),
                        keeper.portrait.as_ref(),
                    )
                })
                .collect(),
            small: false,
        });
    }
    if !pages.wares.is_empty() {
        page.push(Block::Rows {
            rows: pages
                .wares
                .iter()
                .map(|ware| Row {
                    label: ware.name.clone(),
                    value: Some(ware.price.clone()),
                    because: None,
                    change: None,
                    mark: None,
                    picture: None,
                    action: None,
                })
                .collect(),
        });
    }
    let mut panels = blocks(page);
    if carries_nothing {
        panels.empty = Some(empty(
            "Your bag is empty",
            "What you buy, find or make shows up here.",
        ));
    }
    panels
}

fn item(input: &PagesInput<'_>, id: &str) -> PhonePanels {
    let Some(item) = input.pages.items.iter().find(|item| item.id == id) else {
        let mut panels = blocks(vec![back_button("Back to the bag", BAG_PAGE)]);
        panels.empty = Some(empty("Not in your bag", "You no longer carry this."));
        return panels;
    };
    let mut page = match &item.picture {
        Some(picture) => vec![Block::Inspect {
            picture: picture.clone(),
            title: item.name.clone(),
            rarity: None,
            text: item.look.clone(),
        }],
        None => vec![
            heading(&item.name, None),
            Block::Paragraph {
                text: item.look.clone(),
            },
        ],
    };
    let mut rows = vec![Row {
        label: "You carry".to_owned(),
        value: Some(count_text(item.count)),
        because: Some(
            if item.kept {
                "Using it does not use it up."
            } else {
                "Using it uses it up."
            }
            .to_owned(),
        ),
        change: None,
        mark: None,
        picture: None,
        action: None,
    }];
    for (label, value) in [("Weight", &item.weight), ("Packed", &item.stowed)] {
        if let Some(value) = value {
            rows.push(Row {
                label: label.to_owned(),
                value: Some(value.clone()),
                because: None,
                change: None,
                mark: None,
                picture: None,
                action: None,
            });
        }
    }
    page.push(Block::Rows { rows });
    if let Some(worth) = &item.worth {
        page.push(notice(PillTone::Neutral, &format!("Worth {worth}")));
    }
    page.push(Block::Buttons {
        buttons: bag_buttons(input, &item.id),
    });
    page.extend(problem(input));
    page.push(back_button("Back to the bag", BAG_PAGE));
    blocks(page)
}

/// What the hands can do with an item: use it, drop it, and give or sell it to the one the player
/// speaks to (on Talk), when they speak to someone.
fn bag_buttons(input: &PagesInput<'_>, item: &str) -> Vec<ButtonSpec> {
    let mut buttons = vec![
        button(
            "Use",
            ButtonKind::Primary,
            PageTap::Use(item.to_owned()).action(),
        ),
        button(
            "Drop",
            ButtonKind::Secondary,
            PageTap::Drop(item.to_owned()).action(),
        ),
    ];
    let addressed = input
        .ui
        .addressing
        .as_deref()
        .and_then(|id| person_named(input, id));
    match addressed {
        Some(person) => {
            buttons.push(button(
                &format!("Give to {}", person.name),
                ButtonKind::Secondary,
                PageTap::Give(item.to_owned()).action(),
            ));
            buttons.push(button(
                &format!("Sell to {}", person.name),
                ButtonKind::Secondary,
                PageTap::Sell(item.to_owned()).action(),
            ));
        }
        None => buttons.push(ButtonSpec {
            label: "Give or sell".to_owned(),
            kind: ButtonKind::Secondary,
            enabled: false,
            reason: Some("Pick who you speak to on Talk to give or sell it.".to_owned()),
            action: None,
        }),
    }
    buttons
}

/// The shops here as cards, each opening its page.
pub fn shop_cards(pages: &SeatPages) -> Vec<PickCard> {
    pages
        .shops
        .iter()
        .map(|shop| PickCard {
            action: Some(page_action(&shop_page(&shop.id))),
            ..card(
                &format!("Shop with {}", shop.keeper),
                Some(match shop.lines.len() {
                    1 => "1 ware".to_owned(),
                    count => format!("{count} wares"),
                }),
                None,
            )
        })
        .collect()
}

/// A shop's page: its wares with price, weight and stock, Buy on each the shop sells by a tap,
/// and the hero's purse and load to buy against. A ware off the catalog is haggled for on Talk.
fn shop(input: &PagesInput<'_>, id: &str) -> PhonePanels {
    let pages = input.pages;
    let Some(shop) = pages.shops.iter().find(|shop| shop.id == id) else {
        let mut panels = blocks(vec![back_button("Back to Act", "act")]);
        panels.empty = Some(empty("No shop here", "This shop is not where you are now."));
        return panels;
    };
    let mut page = vec![heading(
        &format!("{}'s wares", shop.keeper),
        Some("Tap Buy, or talk with the keeper to haggle.".to_owned()),
    )];
    if let Some(purse) = &pages.purse {
        page.push(notice(PillTone::Neutral, &format!("Your purse: {purse}")));
    }
    if let Some(load) = &pages.load {
        page.push(load_stats(load));
    }
    page.extend(problem(input));
    let rows = shop
        .lines
        .iter()
        .map(|line| {
            let mut about = Vec::new();
            if let Some(weight) = &line.weight {
                about.push(format!("{weight} each"));
            }
            if line.per > 1 {
                about.push(format!("sold {} at a time", line.per));
            }
            about.push(if line.in_stock == 0 {
                "sold out".to_owned()
            } else {
                format!("{} in stock", line.in_stock)
            });
            let (change, action) = if !line.buyable {
                about.push("haggle with the keeper".to_owned());
                ("Haggle", Some(PageTap::Trade(shop.id.clone()).action()))
            } else if line.in_stock >= u64::from(line.per) {
                let buy = PageTap::Buy {
                    shop: shop.id.clone(),
                    item: line.item.clone(),
                    qty: line.per,
                };
                ("Buy", Some(buy.action()))
            } else {
                ("Buy", None)
            };
            Row {
                label: line.name.clone(),
                value: Some(line.price.clone()),
                because: Some(about.join(" · ")),
                change: action.as_ref().map(|_| change.to_owned()),
                mark: None,
                picture: None,
                action,
            }
        })
        .collect::<Vec<_>>();
    let nothing = rows.is_empty();
    if !nothing {
        page.push(Block::Rows { rows });
    }
    page.push(back_button("Back to Act", "act"));
    let mut panels = blocks(page);
    if nothing {
        panels.empty = Some(empty(
            "Nothing for sale",
            "The shop has nothing left to sell.",
        ));
    }
    panels
}

/// The name a line's speaker is shown by: the hero's own, a person's learned name, or the story's.
fn speaker_of(input: &PagesInput<'_>, line: &StoryLine) -> (String, BubbleTone) {
    if line.you {
        return (input.hero.to_owned(), BubbleTone::Hero);
    }
    // The host labels each speaker by the name the hero knows them by.
    match &line.speaker {
        Some(label) => (label.clone(), BubbleTone::Npc),
        None => ("The story".to_owned(), BubbleTone::Narrator),
    }
}

fn person_named<'a>(input: &'a PagesInput<'_>, id: &str) -> Option<&'a Person> {
    input
        .pages
        .people
        .iter()
        .chain(&input.pages.keepers)
        .find(|person| person.id == id)
}

/// Who the next words go to, in words.
fn addressing_text(input: &PagesInput<'_>) -> String {
    match input
        .ui
        .addressing
        .as_deref()
        .and_then(|id| person_named(input, id))
    {
        Some(person) => format!("Speaking to {}", person.name),
        None => "Speaking to the room".to_owned(),
    }
}

/// What a picker of whom to speak to calls the room: no one in particular.
pub const ROOM_LABEL: &str = "No one in particular";

/// Everyone the hero may speak to: the people here and the keepers, each once. The host never
/// lists the seat's own character among them.
pub fn speakable_of(pages: &SeatPages) -> Vec<&Person> {
    let mut people: Vec<&Person> = Vec::new();
    for person in pages.people.iter().chain(&pages.keepers) {
        if !people.iter().any(|seen| seen.id == person.id) {
            people.push(person);
        }
    }
    people
}

fn speakable<'a>(input: &'a PagesInput<'_>) -> Vec<&'a Person> {
    speakable_of(input.pages)
}

fn talk(input: &PagesInput<'_>) -> PhonePanels {
    let thread = input
        .lines
        .iter()
        .map(|line| {
            let (speaker, voice) = speaker_of(input, line);
            ThreadLine {
                speaker,
                voice,
                text: line.text.clone(),
            }
        })
        .collect::<Vec<_>>();
    let people = speakable(input);
    let mut page = vec![heading("Who is here", Some(addressing_text(input)))];
    let mut cards = vec![PickCard {
        selected: input.ui.addressing.is_none(),
        action: Some(PageTap::Address(None).action()),
        ..card(ROOM_LABEL, Some("Everyone here".to_owned()), None)
    }];
    cards.extend(people.iter().map(|person| {
        // Their face cut from the room picture on the shared screen, else their sheet.
        let face = input.faces.iter().find(|(id, _)| *id == person.id);
        PickCard {
            selected: input.ui.addressing.as_deref() == Some(person.id.as_str()),
            action: Some(PageTap::Address(Some(person.id.clone())).action()),
            crop: face.map(|(_, face)| face.crop),
            ..card(
                &person.name,
                person.about.clone(),
                face.map(|(_, face)| &face.media)
                    .or(person.portrait.as_ref()),
            )
        }
    }));
    page.push(Block::Cards { cards, small: true });
    if people.is_empty() {
        page.push(notice(
            PillTone::Neutral,
            "No one else is here yet. When someone is near, they show up by the name you learn.",
        ));
    }
    // A line sent is said when the table next takes talk: until then it waits, as the page says.
    if input.talk_waiting > 0 {
        let waiting = match input.talk_waiting {
            1 => "1 line waiting for the next moment".to_owned(),
            count => format!("{count} lines waiting for the next moment"),
        };
        page.push(notice(PillTone::Neutral, &waiting));
    }
    page.push(Block::Field {
        label: "Your words".to_owned(),
        value: input.ui.talk_draft.clone(),
        placeholder: "Say something".to_owned(),
        message: None,
        tall: false,
        id: Some(TALK_FIELD_ID.to_owned()),
    });
    if input.mic == TalkMic::Recording {
        page.push(notice(
            PillTone::Neutral,
            "Listening. Tap Stop when you are done.",
        ));
    }
    let mut buttons = vec![button(
        "Say it",
        ButtonKind::Primary,
        PageTap::Send.action(),
    )];
    buttons.extend(mic_buttons(input.mic));
    page.push(Block::Buttons { buttons });
    page.extend(problem(input));
    PhonePanels {
        talk: (!thread.is_empty()).then_some(TalkPanel {
            cast: Vec::new(),
            addressing: None,
            thread,
        }),
        ..blocks(page)
    }
}

/// The Talk page's microphone and the seat's switch for talking by voice; none when the table
/// cannot write speech down.
fn mic_buttons(mic: TalkMic) -> Vec<ButtonSpec> {
    let speak = |label: &str| button(label, ButtonKind::Secondary, PageTap::Mic.action());
    let turn_off = button(
        "Turn voice off",
        ButtonKind::Secondary,
        PageTap::Speaking(false).action(),
    );
    match mic {
        TalkMic::Unavailable => Vec::new(),
        TalkMic::Off => vec![button(
            "Turn voice on",
            ButtonKind::Secondary,
            PageTap::Speaking(true).action(),
        )],
        TalkMic::Ready => vec![speak("Speak"), turn_off],
        TalkMic::Recording => vec![speak("Stop"), turn_off],
        TalkMic::Busy => vec![
            ButtonSpec {
                enabled: false,
                reason: Some("Working on the last one...".to_owned()),
                ..speak("Speak")
            },
            turn_off,
        ],
    }
}

/// A kept thing as a card: its kind under the title where kinds are mixed (the notebook), none
/// where every card is the same kind (the paintings).
fn kept_card(kept: &Kept, show_kind: bool) -> PickCard {
    PickCard {
        action: (kept.kind == KeptKind::Painting).then(|| page_action(&painting_page(&kept.id))),
        ..card(
            &kept.title,
            show_kind.then(|| kept.kind.label().to_owned()),
            kept.picture.as_ref(),
        )
    }
}

fn notes(input: &PagesInput<'_>) -> PhonePanels {
    let filter = input.ui.filter;
    let mut page = vec![Block::Chips {
        label: "Show".to_owned(),
        chips: NotesFilter::CHOICES
            .into_iter()
            .map(|choice| Chip {
                text: choice.label().to_owned(),
                selected: choice == filter,
                removable: false,
                action: Some(PageTap::Filter(choice).action()),
            })
            .collect(),
    }];
    let nothing = if filter == NotesFilter::Recent {
        let rows = input
            .lines
            .iter()
            .rev()
            .map(|line| Row {
                label: speaker_of(input, line).0,
                value: Some(line.text.clone()),
                because: None,
                change: Some("Keep".to_owned()),
                mark: None,
                picture: None,
                action: Some(PageTap::Keep(line.id).action()),
            })
            .collect::<Vec<_>>();
        let nothing = rows.is_empty();
        if !nothing {
            page.push(Block::Rows { rows });
        }
        nothing
    } else {
        let chosen = input
            .pages
            .kept
            .iter()
            .filter(|kept| match filter {
                NotesFilter::Kind(kind) => kept.kind == kind,
                NotesFilter::All | NotesFilter::Recent => true,
            })
            .collect::<Vec<_>>();
        let nothing = chosen.is_empty();
        let (pictured, words): (Vec<&Kept>, Vec<&Kept>) =
            chosen.into_iter().partition(|kept| kept.picture.is_some());
        if !pictured.is_empty() {
            page.push(Block::Cards {
                cards: pictured
                    .into_iter()
                    .map(|kept| kept_card(kept, true))
                    .collect(),
                small: true,
            });
        }
        if !words.is_empty() {
            page.push(Block::Rows {
                rows: words
                    .into_iter()
                    .map(|kept| Row {
                        label: kept.title.clone(),
                        value: Some(kept.text.clone()),
                        because: None,
                        change: None,
                        mark: None,
                        picture: None,
                        action: None,
                    })
                    .collect(),
            });
        }
        nothing
    };
    page.push(Block::Field {
        label: "A note".to_owned(),
        value: input.ui.note_draft.clone(),
        placeholder: "What do you want to remember?".to_owned(),
        message: Some("Only you see your notebook.".to_owned()),
        tall: false,
        id: Some(NOTE_FIELD_ID.to_owned()),
    });
    page.push(Block::Buttons {
        buttons: vec![
            button(
                "Keep this note",
                ButtonKind::Primary,
                PageTap::Write.action(),
            ),
            button(
                "Snap the main screen",
                ButtonKind::Secondary,
                PageTap::Snap.action(),
            ),
        ],
    });
    page.extend(problem(input));
    let mut panels = blocks(page);
    if nothing {
        panels.empty = Some(empty(
            "Nothing here yet",
            "Places you reach, paintings you finish, snaps and notes you keep show up here.",
        ));
    }
    panels
}

/// Where the map's `index`th place of `count` sits: a path winding up from the bottom.
fn place_point(index: usize, count: usize) -> (u8, u8) {
    const COLUMNS: [u8; 4] = [22, 50, 78, 50];
    let span = count.saturating_sub(1).max(1);
    let rise = u8::try_from((index * 52 / span).min(52)).unwrap_or(52);
    (COLUMNS[index % COLUMNS.len()], 88 - rise)
}

/// A name that no node before it has, so a way finds its two ends.
fn unique_name(nodes: &[MapNode], name: &str) -> String {
    let taken = |candidate: &str| nodes.iter().any(|node| node.name == candidate);
    if !taken(name) {
        return name.to_owned();
    }
    (2..)
        .map(|number| format!("{name} ({number})"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| name.to_owned())
}

fn map(input: &PagesInput<'_>) -> PhonePanels {
    let pages = input.pages;
    let mut nodes: Vec<MapNode> = Vec::new();
    let mut ways: Vec<MapWay> = Vec::new();
    let mut here: Option<String> = None;
    for (index, place) in pages.places.iter().enumerate() {
        let (x, y) = place_point(index, pages.places.len());
        let name = unique_name(&nodes, &place.name);
        if let Some(before) = nodes.last() {
            ways.push(MapWay {
                from: before.name.clone(),
                to: name.clone(),
                blocked: false,
            });
        }
        if place.here {
            here = Some(name.clone());
        }
        nodes.push(MapNode {
            name,
            x,
            y,
            plate: place.plate.clone(),
            state: if place.here {
                NodeState::Here
            } else {
                NodeState::Visited
            },
            // A place a card goes back to sends that card; any other is told why it cannot.
            action: (!place.here).then(|| match place.card {
                Some(card) => PageTap::Say(card).action(),
                None => PageTap::Refused(place.id.clone()).action(),
            }),
        });
    }
    // A way that goes back to a place on the map is that place's tap, not a new place.
    let back: Vec<usize> = pages.places.iter().filter_map(|place| place.card).collect();
    let onward: Vec<&WayCard> = input
        .ways
        .iter()
        .filter(|way| !back.contains(&way.index))
        .collect();
    let count = onward.len();
    for (index, way) in onward.into_iter().enumerate() {
        let name = unique_name(&nodes, &way.label);
        if let Some(from) = &here {
            ways.push(MapWay {
                from: from.clone(),
                to: name.clone(),
                blocked: false,
            });
        }
        let x = u8::try_from((index + 1) * 100 / (count + 1)).unwrap_or(50);
        nodes.push(MapNode {
            name,
            x,
            y: 12,
            plate: None,
            state: NodeState::Unvisited,
            action: Some(PageTap::Say(way.index).action()),
        });
    }
    let caption = pages
        .place
        .as_ref()
        .map(|place| format!("You are here: {place}"));
    let mut page = vec![heading("Where you have been", caption)];
    if nodes.is_empty() {
        let mut panels = blocks(page);
        panels.empty = Some(empty(
            "No places yet",
            "The places you reach show up here, with the ways on from where you are.",
        ));
        return panels;
    }
    page.push(Block::Map { nodes, ways });
    if let Some(reason) = &input.ui.refused {
        page.push(notice(PillTone::Warn, reason));
    }
    page.push(if input.ways.is_empty() {
        Block::Paragraph {
            text: "When the table asks where you go next, the ways from here show on the map."
                .to_owned(),
        }
    } else {
        heading(
            "Tap a way to go",
            Some("Or say where you want to go on the Act page.".to_owned()),
        )
    });
    page.extend(problem(input));
    blocks(page)
}

/// The button that takes the clue the shared screen shows off it: any phone may, as the table is
/// shared.
fn take_off_button() -> ButtonSpec {
    button(
        "Take it off the table",
        ButtonKind::Secondary,
        PageTap::TakeClueOff.action(),
    )
}

/// The Clues page: every note of the shared screen's clue board, newest first, as a card with its
/// picture, name and what was found of it, each a tap that opens it full screen; and what the
/// table shows full screen now, with the way to take it off.
fn clues(input: &PagesInput<'_>) -> PhonePanels {
    let mut page = vec![heading(
        "Clues",
        Some("What the table has found. Tap one to look at it closely.".to_owned()),
    )];
    if let (Some(board), Some(id)) = (input.clues, input.on_table)
        && let Some(shown) = board.note(id)
    {
        page.push(notice(
            PillTone::Accent,
            &format!("The main screen shows {} now.", shown.name),
        ));
        page.push(Block::Buttons {
            buttons: vec![take_off_button()],
        });
    }
    if input.ui.clue.is_none() {
        page.extend(problem(input));
    }
    let cards: Vec<PickCard> = input
        .clues
        .map(|board| {
            board
                .placed()
                .into_iter()
                .map(|placed| {
                    let note = placed.note;
                    let mark = if input.on_table == Some(note.id.as_str()) {
                        Some(Mark {
                            text: "On the table".to_owned(),
                            tone: PillTone::Accent,
                        })
                    } else {
                        note.worked_out.then(|| Mark {
                            text: "Worked out".to_owned(),
                            tone: PillTone::Good,
                        })
                    };
                    PickCard {
                        mark,
                        action: Some(PageTap::Clue(note.id.clone()).action()),
                        ..card(
                            &note.name,
                            Some(note.detail.clone()).filter(|detail| !detail.is_empty()),
                            note.picture.as_ref(),
                        )
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let nothing = cards.is_empty();
    if !nothing {
        page.push(Block::Cards {
            cards,
            small: false,
        });
    }
    let mut panels = blocks(page);
    if nothing {
        panels.empty = Some(empty(
            "No clues yet",
            "What the table finds shows up here, to look at closely or show on the table.",
        ));
    }
    panels
}

/// The clue open full screen on the phone: the look studio's viewer on it, what was found of it,
/// and the last refusal to show.
#[derive(Debug, Clone, PartialEq)]
pub struct ClueViewer {
    pub id: String,
    pub viewer: LookViewer,
    /// What was found of the clue, or what it was worked out to mean.
    pub text: String,
    pub problem: Option<String>,
}

/// The viewer on the clue `open` of `board`, stepping through the clues in the board's order (the
/// newest first); none when the clue is no longer on the board. It offers to show the clue on the
/// shared screen, and to take off the one the screen shows (`on_table`).
pub fn clue_viewer(
    board: &ClueBoard,
    open: &str,
    on_table: Option<&str>,
    problem: Option<&str>,
) -> Option<ClueViewer> {
    let placed = board.placed();
    let index = placed.iter().position(|placed| placed.note.id == open)?;
    let note = placed.get(index)?.note;
    let step = |at: Option<usize>| {
        at.and_then(|at| placed.get(at))
            .map(|placed| PageTap::Clue(placed.note.id.clone()).action())
    };
    let mut buttons = Vec::new();
    if on_table != Some(open) {
        buttons.push(button(
            "Show on the table",
            ButtonKind::Primary,
            PageTap::ShowClue(note.id.clone()).action(),
        ));
    }
    if on_table.is_some() {
        buttons.push(take_off_button());
    }
    let label = if note.worked_out {
        format!("{} (worked out)", note.name)
    } else {
        note.name.clone()
    };
    Some(ClueViewer {
        id: note.id.clone(),
        viewer: LookViewer {
            item: ViewItem {
                picture: note.picture.clone().unwrap_or_default(),
                label,
                buttons,
            },
            index,
            count: placed.len(),
            previous: step(index.checked_sub(1)),
            next: step(index.checked_add(1)),
            close: PageTap::CloseClue.action(),
        },
        text: note.detail.clone(),
        problem: problem.map(str::to_owned),
    })
}

fn look(input: &PagesInput<'_>) -> PhonePanels {
    let pages = input.pages;
    let mut page = Vec::new();
    if let Some(insert) = &pages.insert {
        page.push(Block::Inspect {
            picture: insert.picture.clone(),
            title: insert
                .caption
                .clone()
                .unwrap_or_else(|| "A close look".to_owned()),
            rarity: None,
            text: String::new(),
        });
        page.push(notice(PillTone::Accent, "The main screen shows this now."));
    }
    page.push(heading("What is in sight", pages.place.clone()));
    let cards = pages
        .things
        .iter()
        .map(|thing| {
            let step = input.looks.iter().find(|look| look.thing == thing.id);
            PickCard {
                mark: step.map(|_| Mark {
                    text: "Step up".to_owned(),
                    tone: PillTone::Accent,
                }),
                action: step.map(|look| PageTap::Say(look.index).action()),
                ..card(&thing.name, thing.look.clone(), thing.picture.as_ref())
            }
        })
        .collect::<Vec<_>>();
    let nothing = cards.is_empty();
    if !nothing {
        page.push(Block::Cards {
            cards,
            small: false,
        });
        if input.looks.is_empty() {
            page.push(notice(
                PillTone::Neutral,
                "When the table asks what you step up to, tap a thing here.",
            ));
        }
    }
    page.extend(problem(input));
    let mut panels = blocks(page);
    if nothing && pages.insert.is_none() {
        panels.empty = Some(empty(
            "Nothing to look at closely",
            "The things in sight here show up when you arrive somewhere.",
        ));
    }
    panels
}

fn paintings(pages: &SeatPages) -> impl Iterator<Item = &Kept> {
    pages
        .kept
        .iter()
        .filter(|kept| kept.kind == KeptKind::Painting)
}

/// Whether the seat's pages carry paintings: one kept, or the day's show up. The "more" sheet
/// offers the paintings only then.
pub fn has_paintings(pages: &SeatPages) -> bool {
    pages.show || paintings(pages).next().is_some()
}

fn painting(input: &PagesInput<'_>, id: &str) -> PhonePanels {
    let Some(kept) = paintings(input.pages).find(|kept| kept.id == id) else {
        let mut panels = blocks(vec![back_button("All paintings", GALLERY_PAGE)]);
        panels.empty = Some(empty("Not found", "This painting is not in your notebook."));
        return panels;
    };
    let mut page = match &kept.picture {
        Some(picture) => vec![Block::Inspect {
            picture: picture.clone(),
            title: kept.title.clone(),
            rarity: None,
            text: kept.text.clone(),
        }],
        None => vec![heading(&kept.title, Some(kept.text.clone()))],
    };
    page.push(notice(PillTone::Good, "Kept in your notebook."));
    page.push(Block::Buttons {
        buttons: vec![
            button(
                "All paintings",
                ButtonKind::Secondary,
                page_action(GALLERY_PAGE),
            ),
            button("Back", ButtonKind::Secondary, page_action("back")),
        ],
    });
    blocks(page)
}

fn gallery(input: &PagesInput<'_>) -> PhonePanels {
    let caption = input
        .pages
        .show
        .then(|| "Your day's work is shown on the main screen.".to_owned());
    let cards = paintings(input.pages)
        .map(|kept| kept_card(kept, false))
        .collect::<Vec<_>>();
    let nothing = cards.is_empty();
    let mut page = vec![heading("Your paintings", caption)];
    if !nothing {
        page.push(Block::Cards {
            cards,
            small: false,
        });
    }
    let mut panels = blocks(page);
    if nothing {
        panels.empty = Some(empty(
            "No paintings yet",
            "Each study you finish shows up here.",
        ));
    }
    panels
}

/// The newest painting kept after `seen` (a memory `seq`), the one to reveal on the phone.
pub fn new_painting(pages: &SeatPages, seen: u64) -> Option<&Kept> {
    pages
        .kept
        .iter()
        .filter(|kept| kept.kind == KeptKind::Painting && kept.seq > seen)
        .max_by_key(|kept| kept.seq)
}

/// The highest `seq` the pages keep: what the phone has seen once it has drawn them.
pub fn newest_seq(pages: &SeatPages) -> u64 {
    pages.kept.iter().map(|kept| kept.seq).max().unwrap_or(0)
}
