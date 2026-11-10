//! The `world inventory` verbs of the dev command line: their grammar, the request each sends and
//! the text of each answer. Free text (a name, a look, a reason) is one word, so quote it:
//! `why="used on the sketch"`.

use std::path::{Path, PathBuf};

use super::request::{
    AddRequest, BatchRequest, Changed, History, InventoryReply, InventoryView, OwnerList,
    SetRequest, TakeRequest, TransferRequest,
};
use super::routes::path;
use super::{Inventory, InventoryChange, InventoryItem, ItemUse, OwnerKind};
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{self, get_json, post_json};

/// Every `world inventory` verb, for the unknown-verb refusal and the help.
pub const INVENTORY_USAGE: &str = concat!(
    "world inventory [list] | world inventory get <owner> | ",
    "world inventory set <owner> <character|party|container|shop> <items.json> | ",
    "world inventory add <owner> <item> <count> [name=<text>] [look=<text>] ",
    "[use=<consumed|kept>] [why=<text>] | ",
    "world inventory remove <owner> <item> <count> [why=<text>] | ",
    "world inventory consume <owner> <item> <count> [why=<text>] | ",
    "world inventory batch <owner> <changes.json> | ",
    "world inventory transfer <from> <to> <item> <count> [why=<text>] | ",
    "world inventory history [<owner>] [limit=<n>] | world inventory sheets <owner>"
);

/// A `world inventory` command of the dev CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryCommand {
    /// Every owner.
    List,
    /// One owner's inventory.
    Get {
        owner: String,
    },
    /// Seed or replace an owner's whole inventory from a JSON file of items.
    Set {
        owner: String,
        kind: OwnerKind,
        file: PathBuf,
    },
    Add(AddRequest),
    Remove(TakeRequest),
    Consume(TakeRequest),
    /// Several changes (a JSON file of `InventoryChange`s) to one owner, all or none.
    Batch {
        owner: String,
        file: PathBuf,
    },
    Transfer(TransferRequest),
    /// The change lines of one owner (all when none).
    History {
        owner: Option<String>,
        limit: Option<usize>,
    },
    /// Ask for the reference sheet of each item of the owner the media library does not hold.
    Sheets {
        owner: String,
    },
}

fn count_word(text: &str) -> Result<u64, CommandError> {
    text.parse::<u64>()
        .map_err(|_| bad_choice("count", text, "a whole number from 0"))
}

fn kind_word(text: &str) -> Result<OwnerKind, CommandError> {
    OwnerKind::from_key(text)
        .ok_or_else(|| bad_choice("kind", text, "character, party, container or shop"))
}

/// The `key=value` words of `words`, each key one of `keys` and given once.
fn pairs<'a>(words: &'a [String], keys: &[&str]) -> Result<Vec<(&'a str, &'a str)>, CommandError> {
    let mut taken: Vec<(&str, &str)> = Vec::new();
    for word in words {
        let (key, value) = word
            .split_once('=')
            .filter(|(key, _)| keys.contains(key))
            .ok_or(CommandError::Usage(INVENTORY_USAGE))?;
        if taken.iter().any(|(seen, _)| *seen == key) {
            return Err(CommandError::DuplicateKey(key.to_owned()));
        }
        taken.push((key, value));
    }
    Ok(taken)
}

fn value_of<'a>(taken: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    taken
        .iter()
        .find(|(seen, _)| *seen == key)
        .map(|(_, value)| *value)
}

/// Reads the words after `world inventory`. The owner and item words are checked by the host, which
/// answers a typed error for each.
pub fn parse(words: &[String]) -> Result<InventoryCommand, CommandError> {
    let Some((verb, rest)) = words.split_first() else {
        return Ok(InventoryCommand::List);
    };
    let usage = || CommandError::Usage(INVENTORY_USAGE);
    match (verb.as_str(), rest) {
        ("list", []) => Ok(InventoryCommand::List),
        ("get", [owner]) => Ok(InventoryCommand::Get {
            owner: owner.clone(),
        }),
        ("set", [owner, kind, file]) => Ok(InventoryCommand::Set {
            owner: owner.clone(),
            kind: kind_word(kind)?,
            file: PathBuf::from(file),
        }),
        ("add", [owner, item, count, tail @ ..]) => {
            let taken = pairs(tail, &["name", "look", "use", "why"])?;
            Ok(InventoryCommand::Add(AddRequest {
                owner: owner.clone(),
                item: item.clone(),
                count: count_word(count)?,
                name: value_of(&taken, "name").map(str::to_owned),
                look: value_of(&taken, "look").map(str::to_owned),
                usage: value_of(&taken, "use")
                    .map(|text| {
                        ItemUse::from_key(text)
                            .ok_or_else(|| bad_choice("use", text, "consumed or kept"))
                    })
                    .transpose()?,
                why: value_of(&taken, "why").unwrap_or_default().to_owned(),
            }))
        }
        ("remove" | "consume", [owner, item, count, tail @ ..]) => {
            let taken = pairs(tail, &["why"])?;
            let request = TakeRequest {
                owner: owner.clone(),
                item: item.clone(),
                count: count_word(count)?,
                why: value_of(&taken, "why").unwrap_or_default().to_owned(),
            };
            Ok(if verb == "remove" {
                InventoryCommand::Remove(request)
            } else {
                InventoryCommand::Consume(request)
            })
        }
        ("batch", [owner, file]) => Ok(InventoryCommand::Batch {
            owner: owner.clone(),
            file: PathBuf::from(file),
        }),
        ("transfer", [from, to, item, count, tail @ ..]) => {
            let taken = pairs(tail, &["why"])?;
            Ok(InventoryCommand::Transfer(TransferRequest {
                from: from.clone(),
                to: to.clone(),
                item: item.clone(),
                count: count_word(count)?,
                why: value_of(&taken, "why").unwrap_or_default().to_owned(),
            }))
        }
        ("history", tail) => {
            let (owner, options) = match tail.split_first() {
                Some((first, options)) if !first.contains('=') => (Some(first.clone()), options),
                _ => (None, tail),
            };
            let taken = pairs(options, &["limit"])?;
            let limit = value_of(&taken, "limit")
                .map(|text| {
                    text.parse::<usize>()
                        .map_err(|_| bad_choice("limit", text, "a whole number from 0"))
                })
                .transpose()?;
            Ok(InventoryCommand::History { owner, limit })
        }
        ("sheets", [owner]) => Ok(InventoryCommand::Sheets {
            owner: owner.clone(),
        }),
        _ => Err(usage()),
    }
}

fn value<T>(reply: InventoryReply<T>) -> Result<T, DevError> {
    dev_client::reply_value(reply)
}

fn format_inventory(inventory: &Inventory) -> String {
    let mut lines = vec![format!(
        "{} ({}) holds {} item(s)",
        inventory.owner,
        inventory.kind,
        inventory.items.len()
    )];
    lines.extend(inventory.items.iter().map(|item| {
        format!(
            "  {} x{}: {} ({})",
            item.id, item.count, item.name, item.look
        )
    }));
    lines.join("\n")
}

fn format_changed(changed: &Changed) -> String {
    let mut lines: Vec<String> = changed
        .applied
        .iter()
        .map(|taken| {
            format!(
                "inventory: {} {} {} {} -> {} ({})",
                taken.owner,
                taken.kind.key(),
                taken.item,
                taken.before,
                taken.after,
                taken.why
            )
        })
        .collect();
    lines.extend(
        changed
            .owners
            .iter()
            .map(|view| format_inventory(&view.inventory)),
    );
    format!("ok\n{}", lines.join("\n"))
}

fn format_owners(list: &OwnerList) -> String {
    let mut lines = vec![format!("owners: {}", list.owners.len())];
    lines.extend(list.owners.iter().map(|owner| {
        format!(
            "  {}  {}  {} item(s), {} unit(s)",
            owner.owner, owner.kind, owner.items, owner.units
        )
    }));
    lines.join("\n")
}

fn format_history(history: &History) -> String {
    if history.entries.is_empty() {
        return "no changes in the host's log".to_owned();
    }
    history
        .entries
        .iter()
        .map(|entry| format!("{:>4}  {:>13}  {}", entry.sequence, entry.at_ms, entry.line))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn file_json<T: serde::de::DeserializeOwned>(file: &Path) -> Result<T, DevError> {
    let text = crate::dev::read_text_file(file).await?;
    serde_json::from_str(&text).map_err(|error| DevError::UnreadableFile {
        path: file.to_path_buf(),
        message: error.to_string(),
    })
}

/// Runs one `world inventory` command against the host on `port`.
pub async fn run(port: u16, command: InventoryCommand) -> Result<String, DevError> {
    match command {
        InventoryCommand::List => {
            let list: OwnerList = value(get_json(port, path::INVENTORY).await?)?;
            Ok(format_owners(&list))
        }
        InventoryCommand::Get { owner } => {
            let address = format!("{}?owner={owner}", path::INVENTORY_OWNER);
            let view: InventoryView = value(get_json(port, &address).await?)?;
            Ok(format_inventory(&view.inventory))
        }
        InventoryCommand::Set { owner, kind, file } => {
            let items: Vec<InventoryItem> = file_json(&file).await?;
            let request = SetRequest { owner, kind, items };
            let changed: Changed = value(post_json(port, path::INVENTORY_SET, &request).await?)?;
            Ok(format_changed(&changed))
        }
        InventoryCommand::Add(request) => post(port, path::INVENTORY_ADD, &request).await,
        InventoryCommand::Remove(request) => post(port, path::INVENTORY_REMOVE, &request).await,
        InventoryCommand::Consume(request) => post(port, path::INVENTORY_CONSUME, &request).await,
        InventoryCommand::Batch { owner, file } => {
            let changes: Vec<InventoryChange> = file_json(&file).await?;
            post(
                port,
                path::INVENTORY_BATCH,
                &BatchRequest { owner, changes },
            )
            .await
        }
        InventoryCommand::Transfer(request) => post(port, path::INVENTORY_TRANSFER, &request).await,
        InventoryCommand::History { owner, limit } => {
            let mut address = path::INVENTORY_HISTORY.to_owned();
            let query: Vec<String> = owner
                .map(|owner| format!("owner={owner}"))
                .into_iter()
                .chain(limit.map(|limit| format!("limit={limit}")))
                .collect();
            if !query.is_empty() {
                address = format!("{address}?{}", query.join("&"));
            }
            let history: History = value(get_json(port, &address).await?)?;
            Ok(format_history(&history))
        }
        InventoryCommand::Sheets { owner } => sheets(port, &owner).await,
    }
}

async fn post<B: serde::Serialize>(port: u16, address: &str, body: &B) -> Result<String, DevError> {
    let changed: Changed = value(post_json(port, address, body).await?)?;
    Ok(format_changed(&changed))
}

/// Asks for the reference sheet of each item `owner` holds that the media library does not hold
/// yet, ahead of play, as a picture that shows the item does.
async fn sheets(port: u16, owner: &str) -> Result<String, DevError> {
    let address = format!("{}?owner={owner}", path::INVENTORY_OWNER);
    let view: InventoryView = value(get_json(port, &address).await?)?;
    let shown: Vec<_> = view
        .inventory
        .items
        .into_iter()
        .map(|item| (owner.to_owned(), item))
        .collect();
    let made = crate::persona::inventory_sheets(port, &shown)
        .await
        .map_err(|(_, error)| *error)?;
    let mut lines: Vec<String> = made
        .sheets
        .iter()
        .filter_map(|sheet| sheet.picture.as_ref())
        .map(|picture| format!("  {picture}: held"))
        .collect();
    lines.extend(
        made.requested
            .iter()
            .map(|name| format!("  {name}: asked for")),
    );
    lines.extend(
        made.not_requested
            .iter()
            .map(|refused| format!("  {refused}: not asked for")),
    );
    Ok(format!(
        "ok sheets of what {owner} holds:\n{}",
        lines.join("\n")
    ))
}
