//! The `world` verbs of the dev command line: their grammar, the request each sends, and the text
//! of each answer. A designer edits `world/bible.json`; these verbs choose the active place and
//! read what the persona prompts would be given.

use df_content::world_bible::{MAXIMUM_ID_CHARS, PriceQuote};

use super::request::{
    PlaceRequest, PlaceSet, Quoted, Validated, WorldReply, WorldView,
};
use super::routes::path;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{self, get_json, post_json};

/// Every `world` verb, for the unknown-verb refusal and the help.
pub const WORLD_USAGE: &str =
    "world [show] | world place <id|-> | world quote <item> <qty> | world validate";
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
    Place { place: Option<String> },
    /// The quote for `qty` of `item` at the active place.
    Quote { item: String, qty: u32 },
    /// Read the bible now and count what it holds.
    Validate,
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
        _ => Err(CommandError::Usage(WORLD_USAGE)),
    }
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

fn format_quote(quoted: &Quoted) -> String {
    let PriceQuote {
        qty,
        ask_each_cp,
        bulk_each_cp,
        discount_permille,
        floor_each_cp,
        total_cp,
        floor_total_cp,
    } = quoted.quote;
    format!(
        "{qty} x {} at {}: {bulk_each_cp} cp each (list {ask_each_cp}, bulk {discount_permille} \
         permille off), asking {total_cp} cp, lowest {floor_total_cp} cp (each {floor_each_cp})",
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
        WorldCommand::Validate => {
            let counts: Validated = reply_value(get_json(port, path::WORLD_VALIDATE).await?)?;
            Ok(format!(
                "ok bible: {} items, {} places, {} stalls, {} voice packs",
                counts.items, counts.places, counts.stalls, counts.voice_packs
            ))
        }
    }
}
