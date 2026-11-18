//! The body and replies of the seat pages routes (`path::PLAY_SEAT_PAGES`, and the notebook and
//! bag actions of `path::PLAY_SEAT_ACTION`), shared by the host that answers them and the phone
//! that reads them.
//!
//! Everything here is the seat's own: the host takes the seat from the key and reads only what
//! the seat's character holds, keeps and may see. Names are the ones the character has learned;
//! someone not yet named is known by what they do. Nothing carries a seat key.

use serde::{Deserialize, Serialize};

use crate::dev_reply::{ErrorTag, HostReply};
use crate::seat::wire::SeatErrorKind;

/// What a kept memory of the seat's character is, by the memory API's kind id (data the runner
/// and the phone agree on, never read from words).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeptKind {
    /// A place the character has been: its plate.
    #[serde(rename = "scene")]
    Place,
    /// A study the character painted.
    #[serde(rename = "artwork")]
    Painting,
    /// What the main screen showed when the player snapped it.
    #[serde(rename = "snap")]
    Snap,
    /// A note the player wrote, or a line they kept.
    #[serde(rename = "note")]
    Note,
}

impl KeptKind {
    pub const ALL: [Self; 4] = [Self::Place, Self::Painting, Self::Snap, Self::Note];

    /// The memory API's kind id.
    pub fn key(self) -> &'static str {
        match self {
            Self::Place => "scene",
            Self::Painting => "artwork",
            Self::Snap => "snap",
            Self::Note => "note",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    /// The word of the Notebook's filter.
    pub fn label(self) -> &'static str {
        match self {
            Self::Place => "Places",
            Self::Painting => "Paintings",
            Self::Snap => "Snaps",
            Self::Note => "My notes",
        }
    }
}

/// Someone in the scene, as the seat's character knows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    /// The persona id: what a tap names, never shown.
    pub id: String,
    /// The learned name, or what they do when the character has not learned it.
    pub name: String,
    /// What they do, under a learned name.
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    pub portrait: Option<String>,
}

/// One thing the character carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BagItem {
    pub id: String,
    pub name: String,
    /// How it looks, in words.
    pub look: String,
    pub count: u64,
    /// Using it does not use it up (a tool, a possession).
    pub kept: bool,
    /// Its reference picture in the media library, when one was made.
    #[serde(default)]
    pub picture: Option<String>,
    /// What it is worth, in words: a coin's denomination ("20 sp"), a ware's price ("2 sp each").
    #[serde(default)]
    pub worth: Option<String>,
    /// What it weighs, in the host's words ("2 lb each, 6 lb"); none when the catalog gives no
    /// weight.
    #[serde(default)]
    pub weight: Option<String>,
    /// Where it is packed ("In the Backpack"); none when it is carried loose.
    #[serde(default)]
    pub stowed: Option<String>,
}

/// What the hero carries by weight, in the host's words: the total against the carrying capacity,
/// and each container's load against its room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BagLoad {
    /// Everything carried, coins included ("58.5 lb").
    pub weight: String,
    /// The carrying capacity ("150 lb"); none when the hero has no sheet to weigh it by.
    #[serde(default)]
    pub capacity: Option<String>,
    /// The hero carries more than the capacity.
    #[serde(default)]
    pub over: bool,
    #[serde(default)]
    pub containers: Vec<ContainerShown>,
    /// The names of what the hero carries that the catalog gives no weight (left out of `weight`).
    #[serde(default)]
    pub unweighed: Vec<String>,
}

/// One container the hero carries: its load against its room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerShown {
    pub id: String,
    pub name: String,
    /// How many of it the hero carries.
    pub count: u64,
    /// What is in it ("12 lb").
    pub load: String,
    /// What it holds at most ("30 lb"); none for a container that holds by count only.
    #[serde(default)]
    pub capacity: Option<String>,
    /// It has no room left by weight or by count.
    #[serde(default)]
    pub full: bool,
    /// What it holds by count, for a quiver or a case ("12 of 20").
    #[serde(default)]
    pub shares: Option<String>,
}

/// A shop here: its keeper and its stock. The shop's id is its keeper's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShopShown {
    pub id: String,
    /// The keeper's name, as the hero knows them.
    pub keeper: String,
    pub lines: Vec<StockShown>,
}

/// One ware of a shop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockShown {
    pub item: String,
    pub name: String,
    pub in_stock: u64,
    /// What `per` of it cost, in the shop's coin ("2 sp", "1 gp for 20").
    pub price: String,
    /// How many one Buy takes: 1, or a bundle (20 arrows).
    pub per: u32,
    /// What one weighs ("2 lb"); none when the catalog gives no weight.
    #[serde(default)]
    pub weight: Option<String>,
    /// Bought by a tap; another ware is bought by talking with the keeper.
    pub buyable: bool,
}

/// What a purchase came to (`SeatAction::Buy`), in the host's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuyDone {
    pub item: String,
    pub qty: u64,
    /// What it cost ("1 gp").
    pub price: String,
    /// What the hero carries now, and their carrying capacity ("62 lb", "150 lb").
    pub weight: String,
    pub capacity: String,
}

/// A ware a keeper here sells, with its price in the keeper's coin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ware {
    /// The keeper who sells it, by persona id.
    pub keeper: String,
    pub id: String,
    pub name: String,
    /// Its price in words ("2 sp each").
    pub price: String,
}

/// What a seat's hands do with a thing it carries, from the Bag (`SeatAction::Bag`). Using and
/// dropping change the bag at once; giving and selling offer terms to someone present, which they
/// take or turn down in their own next line (the deal, settled as one transfer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "act", rename_all = "snake_case", deny_unknown_fields)]
pub enum BagAct {
    /// Use one: used up, unless using it does not use it up (a tool).
    Use { item: String },
    /// Drop one: it leaves the bag.
    Drop { item: String },
    /// Offer one to `to`, a person present, for nothing.
    Give { item: String, to: String },
    /// Offer one to `to`, a person present, for its price in their coin.
    Sell { item: String, to: String },
}

/// What `SeatAction::Bag` came to: what the bag holds of the item now, and whether terms wait
/// for the other's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BagDone {
    pub count: u64,
    pub offered: bool,
}

/// A memory the character keeps, as the Notebook shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    pub id: String,
    pub kind: KeptKind,
    /// The order things happened, only ever up.
    pub seq: u64,
    pub title: String,
    pub text: String,
    #[serde(default)]
    pub picture: Option<String>,
}

/// A place on the map: one the character has been to, by its plate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapPlace {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub plate: Option<String>,
    /// The party is here.
    pub here: bool,
    /// Why a tap cannot go there now; none for where the party is, or where a card goes back.
    #[serde(default)]
    pub refused: Option<String>,
    /// The card of what the table asks now that goes back there (its index), when one does: a
    /// tap sends it, as a tap on the card does.
    #[serde(default)]
    pub card: Option<usize>,
}

/// A thing in sight at the place, worth a close look.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thing {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub look: Option<String>,
    #[serde(default)]
    pub picture: Option<String>,
}

/// A picture the main screen shows now, with its caption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shown {
    pub picture: String,
    #[serde(default)]
    pub caption: Option<String>,
}

/// `path::PLAY_SEAT_PAGES`'s answer: everything the seat's pages show beside its view.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SeatPages {
    /// The name of the place the party is at.
    #[serde(default)]
    pub place: Option<String>,
    /// What the character carries, items with a count of at least one.
    pub items: Vec<BagItem>,
    /// What the character's coins come to, in their coin ("32 sp"), when they carry any.
    #[serde(default)]
    pub purse: Option<String>,
    /// The wares the keepers here sell, with their prices.
    #[serde(default)]
    pub wares: Vec<Ware>,
    /// The keepers of the stalls here, who trade.
    pub keepers: Vec<Person>,
    /// Who else is in the scene.
    pub people: Vec<Person>,
    /// What the character keeps, newest first.
    pub kept: Vec<Kept>,
    /// The places the character has been to, oldest first.
    pub places: Vec<MapPlace>,
    /// The things in sight here.
    pub things: Vec<Thing>,
    /// The close look the main screen cuts to now, when one is up.
    #[serde(default)]
    pub insert: Option<Shown>,
    /// The day's work is being shown on the main screen.
    #[serde(default)]
    pub show: bool,
    /// The seat's hero as the rules derived it (P2), for the Hero and Roll pages; none for a seat
    /// whose hero was not made in creation.
    #[serde(default)]
    pub sheet: Option<HeroSheetView>,
    /// What the hero carries by weight; none before the hero has an inventory.
    #[serde(default)]
    pub load: Option<BagLoad>,
    /// The shops here, each with its stock.
    #[serde(default)]
    pub shops: Vec<ShopShown>,
}

/// One ability of the hero's sheet: its score, modifier and saving throw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetAbility {
    /// The sheet's short key (`str`).
    pub key: String,
    /// The rules' ability key (`strength`): what a roll the table calls names.
    #[serde(default)]
    pub roll_key: String,
    pub name: String,
    pub score: u8,
    pub modifier: i8,
    pub save: i8,
    pub save_proficient: bool,
}

/// One skill of the hero's sheet: what its check adds, and where a proficiency comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetSkill {
    /// The rules' skill key (`athletics`, `animal_handling`): what a skill check names.
    pub key: String,
    pub name: String,
    /// The ability the check adds, by name.
    pub ability: String,
    pub bonus: i8,
    #[serde(default)]
    pub source: Option<String>,
    /// What the hero wears that gives its checks Disadvantage ("Chain Mail"), when something does.
    #[serde(default)]
    pub disadvantage: Option<String>,
    /// Whether the hero has Expertise in the skill (double proficiency; SRD 5.2.1, Rules
    /// Glossary, "Expertise").
    #[serde(default)]
    pub expertise: bool,
}

/// One weapon attack of the hero's sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetAttackLine {
    pub name: String,
    pub bonus: i16,
    /// The damage dice and modifier, as `1d8+3`.
    pub damage: String,
    pub damage_type: String,
}

/// A feature or a trait in play, in a sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetFeature {
    pub name: String,
    pub text: String,
    /// Whether the feature is in play now (`in_play`) or applies once combat arrives
    /// (`applies_in_combat`); none when the sheet does not say.
    #[serde(default)]
    pub status: Option<String>,
}

/// A part of the build designed but not in play yet, shown plainly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetMark {
    pub subject: String,
    pub note: String,
}

/// The hero's sheet as the phone shows it: every number is the server's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeroSheetView {
    pub species: String,
    pub class: String,
    pub background: String,
    pub alignment: String,
    pub level: u8,
    pub hit_points: u32,
    pub armor_class: i16,
    /// How the Armor Class is made ("Chain Mail 16, Defense +1.").
    pub armor_note: String,
    /// None when the species gives no Speed in play.
    #[serde(default)]
    pub speed_feet: Option<u16>,
    pub initiative: i16,
    pub passive_perception: i16,
    pub proficiency_bonus: u8,
    pub abilities: Vec<SheetAbility>,
    pub skills: Vec<SheetSkill>,
    pub attacks: Vec<SheetAttackLine>,
    pub features: Vec<SheetFeature>,
    pub feats: Vec<String>,
    pub languages: Vec<String>,
    pub tools: Vec<String>,
    pub equipment: Vec<String>,
    pub coins_gp: u32,
    pub marks: Vec<SheetMark>,
    /// The hero's live Hit Points; none until the build is submitted.
    #[serde(default)]
    pub current_hp: Option<u32>,
    #[serde(default)]
    pub temp_hp: u32,
    /// The Hit Point Dice left, in words ("1 of 1 d10").
    #[serde(default)]
    pub hit_dice: Option<String>,
    /// The limited features and their uses left.
    #[serde(default)]
    pub uses: Vec<SheetUses>,
    /// The hero's spellcasting, when its class casts spells.
    #[serde(default)]
    pub spellcasting: Option<SheetSpellcasting>,
}

/// A spellcaster's spellcasting as the phone shows it: every number is the server's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetSpellcasting {
    /// The spellcasting ability, by name.
    pub ability: String,
    pub save_dc: i16,
    pub attack_bonus: i16,
    /// How many level 1 and higher spells the hero may have prepared.
    pub prepared_max: u8,
    pub spells: Vec<SheetSpell>,
    pub slots: Vec<SheetSlots>,
}

/// One spell the hero knows. Level 0 is a cantrip, always prepared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetSpell {
    pub id: String,
    pub name: String,
    pub level: u8,
    pub school: String,
    pub ritual: bool,
    pub concentration: bool,
    pub prepared: bool,
    pub casting_time: String,
    pub range: String,
    pub duration: String,
    pub summary: String,
}

/// The spell slots of one level: how many are left of the most the hero has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetSlots {
    pub level: u8,
    pub left: u8,
    pub maximum: u8,
}

/// A limited feature's uses left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetUses {
    pub name: String,
    pub left: u8,
    pub maximum: u8,
}

/// `SeatAction::Note`'s note: what the player keeps in the Notebook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "note", rename_all = "snake_case", deny_unknown_fields)]
pub enum NoteAsk {
    /// A note in the player's own words.
    Write { text: String },
    /// The line of the story the host gave the id `line`, as the host heard it.
    Keep { line: u64 },
    /// What the main screen shows now, made by the host from what it sent it.
    Snap,
}

impl NoteAsk {
    /// The key of the note's kind, for a log line.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Write { .. } => "write",
            Self::Keep { .. } => "keep",
            Self::Snap => "snap",
        }
    }
}

/// What `SeatAction::Note` came to: the memory kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteKept {
    pub id: String,
}

/// Every refusal of the seat pages routes, by the tag of the host's typed error: the host names
/// each of its `PagesError`s by one of these (`PagesError::kind`). A seat refusal reads as the
/// seat routes' own kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagesErrorKind {
    /// The memory did not keep or read the note.
    Memory,
    /// The story has no line of the number.
    UnknownLine,
    /// The shared screen shows no picture to snap.
    NothingShown,
    /// The bag holds none of the item.
    NotCarried,
    /// The item has no price to sell it for.
    NoPrice,
    /// The buyer carries no coin to pay with.
    NoCoin,
    /// The buyer's coins cannot make the price exactly.
    NoExactCoin,
    /// Terms with them are open already.
    OfferOpen,
    /// The terms cannot be offered.
    OfferRefused,
    /// The inventory did not take the change.
    Inventory,
    /// A purchase would take the hero past their carrying capacity.
    OverCapacity,
    /// The hero's containers that could hold the purchase have no room for it.
    ContainerFull,
    /// The hero's coins come to less than the price.
    InsufficientCoin,
    /// The shop has fewer of it than asked.
    NotInStock,
    /// The shop does not sell it by a tap.
    NotForSale,
    /// Nobody here keeps a shop of that id.
    NotAShop,
    /// The shop cannot make the change owed.
    NoChange,
    /// It is sold in bundles of a count.
    UnevenBundle,
    /// The hero has no sheet to weigh the load by.
    NoCarryingCapacity,
    /// A purchase of none.
    ZeroCount,
    /// The coins cannot be counted together.
    CoinMismatch,
    Seat(SeatErrorKind),
}

impl PagesErrorKind {
    /// The pages routes' own kinds (a seat refusal is `SeatErrorKind`'s).
    pub const ALL: [Self; 21] = [
        Self::Memory,
        Self::UnknownLine,
        Self::NothingShown,
        Self::NotCarried,
        Self::NoPrice,
        Self::NoCoin,
        Self::NoExactCoin,
        Self::OfferOpen,
        Self::OfferRefused,
        Self::Inventory,
        Self::OverCapacity,
        Self::ContainerFull,
        Self::InsufficientCoin,
        Self::NotInStock,
        Self::NotForSale,
        Self::NotAShop,
        Self::NoChange,
        Self::UnevenBundle,
        Self::NoCarryingCapacity,
        Self::ZeroCount,
        Self::CoinMismatch,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::UnknownLine => "unknown_line",
            Self::NothingShown => "nothing_shown",
            Self::NotCarried => "not_carried",
            Self::NoPrice => "no_price",
            Self::NoCoin => "no_coin",
            Self::NoExactCoin => "no_exact_coin",
            Self::OfferOpen => "offer_open",
            Self::OfferRefused => "offer_refused",
            Self::Inventory => "inventory",
            Self::OverCapacity => "over_capacity",
            Self::ContainerFull => "container_full",
            Self::InsufficientCoin => "insufficient_coin",
            Self::NotInStock => "not_in_stock",
            Self::NotForSale => "not_for_sale",
            Self::NotAShop => "not_a_shop",
            Self::NoChange => "no_change",
            Self::UnevenBundle => "uneven_bundle",
            Self::NoCarryingCapacity => "no_carrying_capacity",
            Self::ZeroCount => "zero_count",
            Self::CoinMismatch => "coin_mismatch",
            Self::Seat(kind) => kind.key(),
        }
    }

    /// What the phone tells the player of a refusal of this kind.
    pub fn player_words(self) -> &'static str {
        match self {
            Self::Memory => "The notebook could not keep that just now. Try again.",
            Self::UnknownLine => "That line is no longer in the story.",
            Self::NothingShown => "There is no picture on the screen to keep.",
            Self::NotCarried => "You no longer carry that.",
            Self::NoPrice => "That has no price to sell it for.",
            Self::NoCoin => "They have no coin to pay with.",
            Self::NoExactCoin => "Their coins cannot make that price exactly.",
            Self::OfferOpen => "You already offered them something: wait for their answer.",
            Self::OfferRefused => "That cannot be offered now.",
            Self::Inventory => "Your bag did not change. Try again.",
            Self::OverCapacity => "That's more than you can carry.",
            Self::ContainerFull => "There's no room left in your pack.",
            Self::InsufficientCoin => "You don't have enough coin.",
            Self::NotInStock => "They're out of that.",
            Self::NotForSale => "That isn't for sale here.",
            Self::NotAShop => "Nobody here is selling.",
            Self::NoChange => "They can't make change for that.",
            Self::UnevenBundle => "Those are sold a bundle at a time.",
            Self::NoCarryingCapacity => {
                "Your hero sheet isn't ready, so your load can't be weighed."
            }
            Self::ZeroCount => "Choose how many to buy.",
            Self::CoinMismatch => "Those coins can't be counted together. Ask the table.",
            Self::Seat(kind) => kind.player_words(),
        }
    }
}

/// A pages kind is written as its key (`"no_change"`), as the host's typed errors tag it.
impl Serialize for PagesErrorKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

impl<'de> Deserialize<'de> for PagesErrorKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        Self::from_tag(&key).ok_or_else(|| {
            serde::de::Error::custom(format!("'{key}' is not a refusal of the seat pages"))
        })
    }
}

impl ErrorTag for PagesErrorKind {
    fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.key() == tag)
            .or_else(|| SeatErrorKind::from_key(tag).map(Self::Seat))
    }
}

/// A seat pages route's answer as the phone reads it.
pub type PagesReply<T> = HostReply<T, PagesErrorKind>;
