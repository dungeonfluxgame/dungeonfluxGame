//! The `world` verbs of the dev command line: their grammar, the request each sends, and the text
//! of each answer. A designer edits `world/bible.json`; these verbs choose the active place and
//! read what the persona prompts would be given. `world inventory` is the inventory of any owner
//! (`inventory::cli`).

use df_content::world_bible::{MAXIMUM_ID_CHARS, PriceQuote};

use super::inventory::cli::{self as inventory_cli, InventoryCommand};
use super::memory::cli::{self as memory_cli, MemoryCommand};
use super::quest::{
    GoldPaid, Quest, QuestAccept, QuestAccepted, QuestComplete, QuestCompleted, QuestOffer,
    QuestProgress, QuestsView,
};
use super::region::{Region, RegionView};
use super::request::{
    PlaceRequest, PlaceSet, Quoted, ThingAdd, ThingNote, Validated, WorldReply, WorldView,
};
use super::routes::path;
use super::town::TownView;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{self, get_json, post_json};

/// Every `world` verb, for the unknown-verb refusal and the help.
pub const WORLD_USAGE: &str = concat!(
    "world [show] | world place <id|-> | world quote <item> <qty> | world validate | world town | ",
    "world thing add <id> <name...> | <look...> | world arrive <id> <name...> | <in sight...> | ",
    "world quest [show] | world quest offer <quest>... | world quest accept <id> <hero>... | ",
    "world quest progress <index> <total> <place...> | world quest complete <id> | world region | ",
    "world region enter <id> | <title> | <arrival> | <look> | world region card-clear | ",
    "world inventory ... (world inventory help) | world memory ... (world memory help)"
);
/// `world quest`: the quests offered and taken, a board, a quest taken and the stop reached.
pub const WORLD_QUEST_USAGE: &str = concat!(
    "world quest [show] | ",
    "world quest offer <id>|<title>|<giver>|<reward_gp>|<advance_gp>|<site>|<clue,ids>... | ",
    "world quest accept <id> <hero>... | world quest progress <index> <total> <place...> | ",
    "world quest complete <id>"
);
/// `world region`: the region the party is in, or a crossing into another.
pub const WORLD_REGION_USAGE: &str = concat!(
    "world region | world region enter <id> | <title> | <arrival> | <look> | ",
    "world region card-clear"
);
/// `world arrive`: a place in play with no design (a plate painted from words alone) made the
/// active one, with no things yet, so what is seen there can be added to it.
pub const WORLD_ARRIVE_USAGE: &str = "world arrive <id> <name...> | <what is in sight...>";
/// `world thing add`: a thing in sight at the active place designed in play.
pub const WORLD_THING_USAGE: &str = concat!(
    "world thing add <id> <name...> | <look...> | ",
    "world thing note <id> <label> [<new name...>] | <text...>"
);
/// `world place`: the active place (`-` for none).
pub const WORLD_PLACE_USAGE: &str = "world place <id|->";
/// `world quote`: the price of a lot at the active place.
pub const WORLD_QUOTE_USAGE: &str = "world quote <item> <qty>";

/// A world command of the dev CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldCommand {
    /// The bible's ids, the active place and the facts of it.
    Show,
    /// Make `place` the active one; none clears it.
    Place {
        place: Option<String>,
    },
    /// The quote for `qty` of `item` at the active place.
    Quote {
        item: String,
        qty: u32,
    },
    /// Read the bible now and count what it holds.
    Validate,
    /// The town the party is in, if any.
    Town,
    /// A thing in sight added to the active place designed in play.
    ThingAdd(ThingAdd),
    ThingNote(ThingNote),
    /// A place in play made the active one, with no things yet (`POST /dev/world/arrive`).
    Arrive(super::request::ArriveRequest),
    /// An inventory operation.
    Inventory(InventoryCommand),
    /// A memory operation.
    Memory(MemoryCommand),
    /// The quests offered and taken.
    Quests,
    /// Quests put on the board.
    QuestOffer(Vec<Quest>),
    /// An offered quest taken by the heroes.
    QuestAccept(QuestAccept),
    /// The stop the party reached on the quest's way.
    QuestProgress(QuestProgress),
    QuestComplete(QuestComplete),
    /// The region the party is in.
    Region,
    /// A crossing into another region.
    RegionEnter(Region),
    /// The crossing's title card taken off the shared screen.
    RegionCardClear,
}

fn id_word(key: &str, word: &str) -> Result<String, CommandError> {
    if df_content::world_bible::is_valid_id(word) {
        Ok(word.to_owned())
    } else {
        Err(bad_choice(
            key,
            word,
            format!("1 to {MAXIMUM_ID_CHARS} lowercase letters, digits, '_' or '-'"),
        ))
    }
}

/// Reads the words after `world`.
pub fn parse(words: &[String]) -> Result<WorldCommand, CommandError> {
    let Some((verb, rest)) = words.split_first() else {
        return Ok(WorldCommand::Show);
    };
    match (verb.as_str(), rest) {
        ("show", []) => Ok(WorldCommand::Show),
        ("place", [place]) if place == "-" => Ok(WorldCommand::Place { place: None }),
        ("place", [place]) => Ok(WorldCommand::Place {
            place: Some(id_word("place", place)?),
        }),
        ("place", _) => Err(CommandError::Usage(WORLD_PLACE_USAGE)),
        ("quote", [item, qty]) => Ok(WorldCommand::Quote {
            item: id_word("item", item)?,
            qty: qty.parse().map_err(|_| {
                bad_choice("qty", qty, format!("a whole number from 1 to {}", u32::MAX))
            })?,
        }),
        ("quote", _) => Err(CommandError::Usage(WORLD_QUOTE_USAGE)),
        ("validate", []) => Ok(WorldCommand::Validate),
        ("town", []) => Ok(WorldCommand::Town),
        ("thing", [add, id, rest @ ..]) if add == "add" => thing_add(id, rest),
        ("thing", [note, id, label, rest @ ..]) if note == "note" => thing_note(id, label, rest),
        ("thing", _) => Err(CommandError::Usage(WORLD_THING_USAGE)),
        ("arrive", [id, rest @ ..]) => arrive(id, rest),
        ("inventory", words) => inventory_cli::parse(words).map(WorldCommand::Inventory),
        ("memory", words) => memory_cli::parse(words).map(WorldCommand::Memory),
        ("quest", words) => quest(words),
        ("region", []) => Ok(WorldCommand::Region),
        ("region", [enter, rest @ ..]) if enter == "enter" => region_enter(rest),
        ("region", [clear]) if clear == "card-clear" => Ok(WorldCommand::RegionCardClear),
        ("region", _) => Err(CommandError::Usage(WORLD_REGION_USAGE)),
        _ => Err(CommandError::Usage(WORLD_USAGE)),
    }
}

/// The fields of `text` split at `|`, each without the spaces around it.
fn fields(text: &str) -> Vec<String> {
    text.split('|')
        .map(|field| field.trim().to_owned())
        .collect()
}

/// A whole number of gold pieces.
fn gold(key: &str, word: &str) -> Result<u64, CommandError> {
    word.parse()
        .map_err(|_| bad_choice(key, word, "a whole number of gold pieces"))
}

/// One quest as the beats author it:
/// `<id>|<title>|<giver>|<reward_gp>|<advance_gp>|<site>|<clue,ids>`.
fn quest_spec(spec: &str) -> Result<Quest, CommandError> {
    let [id, title, giver, reward, advance, site, clues] = fields(spec)
        .try_into()
        .map_err(|_| CommandError::Usage(WORLD_QUEST_USAGE))?;
    let clues = clues
        .split(',')
        .map(str::trim)
        .filter(|clue| !clue.is_empty())
        .map(|clue| id_word("clue", clue))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Quest {
        id: id_word("quest", &id)?,
        title,
        giver,
        reward_gp: gold("reward_gp", &reward)?,
        advance_gp: gold("advance_gp", &advance)?,
        site,
        clues,
    })
}

/// `world quest ...`: each quest of a board is one word of its own.
fn quest(words: &[String]) -> Result<WorldCommand, CommandError> {
    let usage = || CommandError::Usage(WORLD_QUEST_USAGE);
    match words.split_first() {
        None => Ok(WorldCommand::Quests),
        Some((verb, [])) if verb == "show" => Ok(WorldCommand::Quests),
        Some((verb, specs)) if verb == "offer" && !specs.is_empty() => specs
            .iter()
            .map(|spec| quest_spec(spec))
            .collect::<Result<Vec<_>, _>>()
            .map(WorldCommand::QuestOffer),
        Some((verb, [id, heroes @ ..])) if verb == "accept" && !heroes.is_empty() => {
            Ok(WorldCommand::QuestAccept(QuestAccept {
                quest: id_word("quest", id)?,
                heroes: heroes
                    .iter()
                    .map(|hero| id_word("hero", hero))
                    .collect::<Result<Vec<_>, _>>()?,
            }))
        }
        Some((verb, [index, total, place @ ..])) if verb == "progress" && !place.is_empty() => {
            let number = |key: &str, word: &str| {
                word.parse::<u16>()
                    .map_err(|_| bad_choice(key, word, "a whole number from 1"))
            };
            Ok(WorldCommand::QuestProgress(QuestProgress {
                index: number("index", index)?,
                total: number("total", total)?,
                place: place.join(" "),
            }))
        }
        Some((verb, [id])) if verb == "complete" => {
            Ok(WorldCommand::QuestComplete(QuestComplete {
                quest: id_word("quest", id)?,
            }))
        }
        _ => Err(usage()),
    }
}

/// `world region enter <id> | <title> | <arrival> | <look>`, its words split at `|`.
fn region_enter(rest: &[String]) -> Result<WorldCommand, CommandError> {
    let [id, title, arrival, look] = fields(&rest.join(" "))
        .try_into()
        .map_err(|_| CommandError::Usage(WORLD_REGION_USAGE))?;
    Ok(WorldCommand::RegionEnter(Region {
        id: id_word("region", &id)?,
        title,
        arrival,
        look,
    }))
}

/// `world thing add <id> <name...> | <look...>`: the name and the look are split at the `|` word.
fn arrive(id: &str, rest: &[String]) -> Result<WorldCommand, CommandError> {
    let usage = || CommandError::Usage(WORLD_ARRIVE_USAGE);
    let split = rest.iter().position(|word| word == "|").ok_or_else(usage)?;
    let (name, in_sight) = (rest[..split].join(" "), rest[split + 1..].join(" "));
    if name.trim().is_empty() || in_sight.trim().is_empty() {
        return Err(usage());
    }
    Ok(WorldCommand::Arrive(super::request::ArriveRequest {
        place: df_content::world_bible::Place {
            id: id_word("id", id)?,
            name,
            description: format!("In sight: {in_sight}."),
            stalls: Vec::new(),
            things: Vec::new(),
        },
        feature: None,
        shops: Vec::new(),
    }))
}

fn thing_add(id: &str, rest: &[String]) -> Result<WorldCommand, CommandError> {
    let usage = || CommandError::Usage(WORLD_THING_USAGE);
    let split = rest.iter().position(|word| word == "|").ok_or_else(usage)?;
    let (name, look) = (rest[..split].join(" "), rest[split + 1..].join(" "));
    if name.trim().is_empty() || look.trim().is_empty() {
        return Err(usage());
    }
    Ok(WorldCommand::ThingAdd(ThingAdd {
        id: id_word("id", id)?,
        name,
        look,
    }))
}

/// `world thing note <id> <label> [<new name...>] | <text...>`: split at the `|` word.
fn thing_note(id: &str, label: &str, rest: &[String]) -> Result<WorldCommand, CommandError> {
    let usage = || CommandError::Usage(WORLD_THING_USAGE);
    let split = rest.iter().position(|word| word == "|").ok_or_else(usage)?;
    let (name, text) = (rest[..split].join(" "), rest[split + 1..].join(" "));
    if text.trim().is_empty() {
        return Err(usage());
    }
    Ok(WorldCommand::ThingNote(ThingNote {
        id: id_word("id", id)?,
        label: id_word("label", label)?,
        text,
        name: Some(name).filter(|name| !name.trim().is_empty()),
    }))
}

/// The value of a world reply, or its refusal (the typed `WorldError`) as the command's failure.
fn reply_value<T>(reply: WorldReply<T>) -> Result<T, DevError> {
    dev_client::reply_value(reply)
}

fn format_view(view: &WorldView) -> String {
    let mut lines = vec![format!(
        "active place: {}",
        view.place.as_deref().unwrap_or("none")
    )];
    let listed = |label: &str, rows: &[super::request::Listed]| {
        let mut out = vec![format!("{label}: {}", rows.len())];
        out.extend(rows.iter().map(|row| format!("  {}  {}", row.id, row.text)));
        out.join("\n")
    };
    lines.push(listed("places", &view.places));
    lines.push(format!("items: {}", view.items.join(" ")));
    lines.push(listed("voice packs", &view.voice_packs));
    if !view.facts.is_null() {
        lines.push(format!(
            "facts for the persona prompts:\n{}",
            serde_json::to_string_pretty(&view.facts).unwrap_or_default()
        ));
    }
    lines.join("\n")
}

fn format_town(view: &TownView) -> String {
    match &view.town {
        Some(town) => format!(
            "town id={} created_at={}\nlook: {}",
            town.id, town.created_at, town.look
        ),
        None => "town none".to_owned(),
    }
}

fn format_quests(view: &QuestsView) -> String {
    if view.quests.is_empty() {
        return "quests none".to_owned();
    }
    view.quests
        .iter()
        .map(|record| {
            let quest = &record.quest;
            format!(
                "quest {} {} reward_gp={} advance_gp={} giver={} site={} clues={}{}\n  {}",
                quest.id,
                record.standing.key(),
                quest.reward_gp,
                quest.advance_gp,
                quest.giver,
                quest.site,
                quest.clues.join(","),
                if record.accepted_by.is_empty() {
                    String::new()
                } else {
                    format!(" by={}", record.accepted_by.join(","))
                },
                quest.title
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_accepted(accepted: &QuestAccepted) -> String {
    let mut lines = vec![format!(
        "ok quest {} accepted reward_gp={} advance_gp={}",
        accepted.quest.id, accepted.quest.reward_gp, accepted.quest.advance_gp
    )];
    if !accepted.already.is_empty() {
        lines.push(format!("already_yours {}", accepted.already.join(" ")));
    }
    lines.extend(paid_noted(&accepted.paid, &accepted.noted));
    lines.join("\n")
}

fn format_completed(completed: &QuestCompleted) -> String {
    let mut lines = vec![format!(
        "ok quest {} completed reward_gp={}",
        completed.quest.id, completed.quest.reward_gp
    )];
    lines.extend(paid_noted(&completed.paid, &completed.noted));
    lines.join("\n")
}

/// The lines of what a quest paid each hero and the notes it kept: `paid <hero> <count> <item>`
/// or `pay_refused <hero> <reason>`, then `noted <hero> <memory>`.
fn paid_noted(paid: &[GoldPaid], noted: &[(String, String)]) -> Vec<String> {
    let paid = paid.iter().map(|paid| match paid {
        GoldPaid::Paid { hero, item, count } => format!("paid {hero} {count} {item}"),
        GoldPaid::Refused { hero, reason } => format!("pay_refused {hero} {reason}"),
    });
    let noted = noted
        .iter()
        .map(|(hero, memory)| format!("noted {hero} {memory}"));
    paid.chain(noted).collect()
}

fn format_region(view: &RegionView) -> String {
    let left = view
        .left
        .as_ref()
        .map_or(String::new(), |left| format!(" from={}", left.id));
    match &view.region {
        Some(region) => format!(
            "region id={}{left}\ntitle: {}\narrival: {}\nlook: {}",
            region.id, region.title, region.arrival, region.look
        ),
        None => "region none".to_owned(),
    }
}

fn format_quote(quoted: &Quoted) -> String {
    let PriceQuote {
        qty,
        ask_each_cp,
        discount_permille,
        ask_total_cp,
        tier_total_cp,
        floor_total_cp,
        tier_each_cp,
        floor_each_cp,
    } = quoted.quote;
    format!(
        "{qty} x {} at {}: ask {ask_total_cp} cp (list {ask_each_cp} each), \
         tier {tier_total_cp} cp (bulk {discount_permille} permille off, {tier_each_cp} each), \
         floor {floor_total_cp} cp ({floor_each_cp} each)",
        quoted.item, quoted.place
    )
}

/// Runs one world command against the host on `port`.
pub async fn run(port: u16, command: WorldCommand) -> Result<String, DevError> {
    match command {
        WorldCommand::Show => {
            let view: WorldView = reply_value(get_json(port, path::WORLD).await?)?;
            Ok(format_view(&view))
        }
        WorldCommand::Place { place } => {
            let request = PlaceRequest { place };
            let set: PlaceSet = reply_value(post_json(port, path::WORLD_PLACE, &request).await?)?;
            Ok(format!(
                "ok active place: {}",
                set.place.as_deref().unwrap_or("none")
            ))
        }
        WorldCommand::Quote { item, qty } => {
            let address = format!("{}?item={item}&qty={qty}", path::WORLD_QUOTE);
            let quoted: Quoted = reply_value(get_json(port, &address).await?)?;
            Ok(format_quote(&quoted))
        }
        WorldCommand::Inventory(command) => inventory_cli::run(port, command).await,
        WorldCommand::Memory(command) => memory_cli::run(port, command).await,
        WorldCommand::Arrive(request) => {
            let set: PlaceSet = reply_value(post_json(port, path::WORLD_ARRIVE, &request).await?)?;
            Ok(format!(
                "ok place {} is the active one, with no things yet",
                set.place.as_deref().unwrap_or("none")
            ))
        }
        WorldCommand::ThingNote(note) => {
            let id = note.id.clone();
            let set: PlaceSet = reply_value(post_json(port, path::WORLD_THING_NOTE, &note).await?)?;
            Ok(format!(
                "ok noted {id} at {}",
                set.place.as_deref().unwrap_or("none")
            ))
        }
        WorldCommand::ThingAdd(thing) => {
            let id = thing.id.clone();
            let set: PlaceSet = reply_value(post_json(port, path::WORLD_THING_ADD, &thing).await?)?;
            Ok(format!(
                "ok thing {id} at {}",
                set.place.as_deref().unwrap_or("none")
            ))
        }
        WorldCommand::Quests => {
            let view: QuestsView = reply_value(get_json(port, path::WORLD_QUESTS).await?)?;
            Ok(format_quests(&view))
        }
        WorldCommand::QuestOffer(quests) => {
            let request = QuestOffer { quests };
            let view: QuestsView =
                reply_value(post_json(port, path::WORLD_QUEST_OFFER, &request).await?)?;
            Ok(format_quests(&view))
        }
        WorldCommand::QuestAccept(request) => {
            let accepted: QuestAccepted =
                reply_value(post_json(port, path::WORLD_QUEST_ACCEPT, &request).await?)?;
            Ok(format_accepted(&accepted))
        }
        WorldCommand::QuestProgress(request) => {
            let (index, total) = (request.index, request.total);
            let _: QuestsView =
                reply_value(post_json(port, path::WORLD_QUEST_PROGRESS, &request).await?)?;
            Ok(format!("ok quest progress {index} of {total}"))
        }
        WorldCommand::QuestComplete(request) => {
            let completed: QuestCompleted =
                reply_value(post_json(port, path::WORLD_QUEST_COMPLETE, &request).await?)?;
            Ok(format_completed(&completed))
        }
        WorldCommand::Region => {
            let view: RegionView = reply_value(get_json(port, path::WORLD_REGION).await?)?;
            Ok(format_region(&view))
        }
        WorldCommand::RegionEnter(region) => {
            let view: RegionView =
                reply_value(post_json(port, path::WORLD_REGION, &region).await?)?;
            Ok(format!("ok {}", format_region(&view)))
        }
        WorldCommand::RegionCardClear => {
            let response = dev_client::post(port, path::WORLD_REGION_CARD_CLEAR, "").await?;
            let view: RegionView = reply_value(dev_client::read_answer(&response)?)?;
            Ok(format!("ok card cleared, {}", format_region(&view)))
        }
        WorldCommand::Town => {
            let view: TownView = reply_value(get_json(port, path::WORLD_TOWN).await?)?;
            Ok(format_town(&view))
        }
        WorldCommand::Validate => {
            let counts: Validated = reply_value(get_json(port, path::WORLD_VALIDATE).await?)?;
            Ok(format!(
                "ok bible: {} items, {} places, {} stalls, {} voice packs",
                counts.items, counts.places, counts.stalls, counts.voice_packs
            ))
        }
    }
}
