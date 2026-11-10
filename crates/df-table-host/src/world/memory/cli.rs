//! The `world memory` verbs of the dev command line: their grammar, the request each sends and
//! the text of each answer. Free text (a title, a description) is one word, so quote it:
//! `title="the market square"`. A word that may be given more than once (`tag=`, `id=`, `link=`)
//! is repeated.

use std::path::{Path, PathBuf};

use super::request::{
    CaptureRequest, ComposeRequest, Composed, DeleteRequest, Deleted, History, MemoryPage,
    MemoryReply, OwnerList, ReferencesRequest,
};
use super::routes::path;
use super::{LinkKind, Memory, MemoryDraft, MemoryFilter, MemoryLink, Put, References};
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{self, get_json, post_json};

/// Every `world memory` verb, for the unknown-verb refusal and the help.
pub const MEMORY_USAGE: &str = concat!(
    "world memory [owners] | world memory put <memory.json> | world memory get <owner> <id> | ",
    "world memory list <owner> [kind=<id>] [place=<id>] [tag=<id>] [limit=<n>] [after=<cursor>] | ",
    "world memory delete <owner> <id> | ",
    "world memory capture <owner> <kind> <picture> <title> [place=<id>] [tag=<id>].. [id=<id>] ",
    "[description=<text>] [by=<text>] [link=<item|memory|place>:<id>].. | ",
    "world memory references <owner> <budget> [kind=<id>] [place=<id>] [tag=<id>] [id=<id>].. | ",
    "world memory compose <owner> [name=<id>] [kind=<id>] [place=<id>] [tag=<id>] [id=<id>].. ",
    "[columns=<n>] [tile=<w>x<h>] [gap=<px>] [background=<rrggbb>] | ",
    "world memory history [<owner>] [limit=<n>]"
);

/// A `world memory` command of the dev CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryCommand {
    /// Every owner that keeps memories.
    Owners,
    /// Create or replace a memory from a JSON file (a `MemoryDraft`).
    Put {
        file: PathBuf,
    },
    Get {
        owner: String,
        id: String,
    },
    List {
        owner: String,
        filter: MemoryFilter,
        limit: Option<usize>,
        after: Option<String>,
    },
    Delete(DeleteRequest),
    Capture(CaptureRequest),
    References(ReferencesRequest),
    Compose(ComposeRequest),
    History {
        owner: Option<String>,
        limit: Option<usize>,
    },
}

/// The `key=value` words of `words`, each key one of `keys`, in the order given. A key may repeat.
fn pairs<'a>(words: &'a [String], keys: &[&str]) -> Result<Vec<(&'a str, &'a str)>, CommandError> {
    words
        .iter()
        .map(|word| {
            word.split_once('=')
                .filter(|(key, _)| keys.contains(key))
                .ok_or(CommandError::Usage(MEMORY_USAGE))
        })
        .collect()
}

/// The values of `key`, in the order given.
fn values<'a>(taken: &[(&'a str, &'a str)], key: &str) -> Vec<&'a str> {
    taken
        .iter()
        .filter(|(seen, _)| *seen == key)
        .map(|(_, value)| *value)
        .collect()
}

/// The value of `key`, given at most once.
fn single<'a>(taken: &[(&'a str, &'a str)], key: &str) -> Result<Option<&'a str>, CommandError> {
    let mut found = values(taken, key).into_iter();
    let first = found.next();
    if found.next().is_some() {
        return Err(CommandError::DuplicateKey(key.to_owned()));
    }
    Ok(first)
}

fn number<T: std::str::FromStr>(key: &str, text: &str) -> Result<T, CommandError> {
    text.parse::<T>()
        .map_err(|_| bad_choice(key, text, "a whole number from 0"))
}

fn optional_number<T: std::str::FromStr>(
    taken: &[(&str, &str)],
    key: &str,
) -> Result<Option<T>, CommandError> {
    single(taken, key)?
        .map(|text| number(key, text))
        .transpose()
}

fn text_of(taken: &[(&str, &str)], key: &str) -> Result<Option<String>, CommandError> {
    Ok(single(taken, key)?.map(str::to_owned))
}

fn filter_of(taken: &[(&str, &str)]) -> Result<MemoryFilter, CommandError> {
    Ok(MemoryFilter {
        kind: text_of(taken, "kind")?,
        place: text_of(taken, "place")?,
        tag: text_of(taken, "tag")?,
    })
}

fn link_of(word: &str) -> Result<MemoryLink, CommandError> {
    let (what, id) = word
        .split_once(':')
        .and_then(|(what, id)| LinkKind::from_key(what).map(|what| (what, id)))
        .ok_or_else(|| bad_choice("link", word, "item:<id>, memory:<id> or place:<id>"))?;
    Ok(MemoryLink {
        what,
        id: id.to_owned(),
    })
}

fn background_of(text: &str) -> Result<[u8; 3], CommandError> {
    let bad = || bad_choice("background", text, "six hex digits, rrggbb");
    if text.len() != 6 || !text.is_ascii() {
        return Err(bad());
    }
    let channel = |at: usize| u8::from_str_radix(&text[at..at + 2], 16).map_err(|_| bad());
    Ok([channel(0)?, channel(2)?, channel(4)?])
}

fn tile_of(text: &str) -> Result<(u32, u32), CommandError> {
    let bad = || bad_choice("tile", text, "<width>x<height> in pixels");
    let (width, height) = text.split_once('x').ok_or_else(bad)?;
    Ok((
        width.parse().map_err(|_| bad())?,
        height.parse().map_err(|_| bad())?,
    ))
}

/// Reads the words after `world memory`. The owner, id and picture words are checked by the host,
/// which answers a typed error for each.
pub fn parse(words: &[String]) -> Result<MemoryCommand, CommandError> {
    let Some((verb, rest)) = words.split_first() else {
        return Ok(MemoryCommand::Owners);
    };
    let usage = || CommandError::Usage(MEMORY_USAGE);
    match (verb.as_str(), rest) {
        ("owners", []) => Ok(MemoryCommand::Owners),
        ("put", [file]) => Ok(MemoryCommand::Put {
            file: PathBuf::from(file),
        }),
        ("get", [owner, id]) => Ok(MemoryCommand::Get {
            owner: owner.clone(),
            id: id.clone(),
        }),
        ("list", [owner, tail @ ..]) => {
            let taken = pairs(tail, &["kind", "place", "tag", "limit", "after"])?;
            Ok(MemoryCommand::List {
                owner: owner.clone(),
                filter: filter_of(&taken)?,
                limit: optional_number(&taken, "limit")?,
                after: text_of(&taken, "after")?,
            })
        }
        ("delete", [owner, id]) => Ok(MemoryCommand::Delete(DeleteRequest {
            owner: owner.clone(),
            id: id.clone(),
        })),
        ("capture", [owner, kind, picture, title, tail @ ..]) => {
            let taken = pairs(tail, &["place", "tag", "id", "description", "by", "link"])?;
            Ok(MemoryCommand::Capture(CaptureRequest {
                owner: owner.clone(),
                kind: kind.clone(),
                picture: picture.clone(),
                title: title.clone(),
                id: text_of(&taken, "id")?,
                description: text_of(&taken, "description")?,
                place: text_of(&taken, "place")?,
                links: values(&taken, "link")
                    .into_iter()
                    .map(link_of)
                    .collect::<Result<_, _>>()?,
                tags: values(&taken, "tag")
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                created_by: text_of(&taken, "by")?,
            }))
        }
        ("references", [owner, budget, tail @ ..]) => {
            let taken = pairs(tail, &["kind", "place", "tag", "id"])?;
            Ok(MemoryCommand::References(ReferencesRequest {
                owner: owner.clone(),
                filter: filter_of(&taken)?,
                ids: values(&taken, "id")
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                budget: number("budget", budget)?,
            }))
        }
        ("compose", [owner, tail @ ..]) => {
            let taken = pairs(
                tail,
                &[
                    "name",
                    "kind",
                    "place",
                    "tag",
                    "id",
                    "columns",
                    "tile",
                    "gap",
                    "background",
                ],
            )?;
            let tile = single(&taken, "tile")?.map(tile_of).transpose()?;
            Ok(MemoryCommand::Compose(ComposeRequest {
                owner: owner.clone(),
                filter: filter_of(&taken)?,
                ids: values(&taken, "id")
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                name: text_of(&taken, "name")?,
                columns: optional_number(&taken, "columns")?,
                tile_width_px: tile.map(|(width, _)| width),
                tile_height_px: tile.map(|(_, height)| height),
                gap_px: optional_number(&taken, "gap")?,
                background: single(&taken, "background")?
                    .map(background_of)
                    .transpose()?,
            }))
        }
        ("history", tail) => {
            let (owner, options) = match tail.split_first() {
                Some((first, options)) if !first.contains('=') => (Some(first.clone()), options),
                _ => (None, tail),
            };
            let taken = pairs(options, &["limit"])?;
            Ok(MemoryCommand::History {
                owner,
                limit: optional_number(&taken, "limit")?,
            })
        }
        _ => Err(usage()),
    }
}

fn value<T>(reply: MemoryReply<T>) -> Result<T, DevError> {
    dev_client::reply_value(reply)
}

fn comma(words: &[String]) -> String {
    if words.is_empty() {
        "-".to_owned()
    } else {
        words.join(",")
    }
}

fn format_memory(memory: &Memory) -> String {
    let mut lines = vec![format!(
        "{} ({}) of {}, seq {}: {}",
        memory.id, memory.kind, memory.owner, memory.seq, memory.title
    )];
    lines.push(format!("  look: {}", memory.description));
    lines.push(format!(
        "  place: {}  pictures: {}  tags: {}",
        memory.place.as_deref().unwrap_or("-"),
        comma(&memory.pictures),
        comma(&memory.tags)
    ));
    if !memory.links.is_empty() {
        let links: Vec<String> = memory
            .links
            .iter()
            .map(|link| format!("{}:{}", link.what.key(), link.id))
            .collect();
        lines.push(format!("  links: {}", links.join(",")));
    }
    if let Some(by) = &memory.created_by {
        lines.push(format!("  by: {by}"));
    }
    lines.join("\n")
}

fn format_put(put: &Put) -> String {
    format!(
        "ok memory={} kind={} seq={} created={}\n{}",
        put.memory.id,
        put.memory.kind,
        put.memory.seq,
        put.created,
        format_memory(&put.memory)
    )
}

fn format_owners(list: &OwnerList) -> String {
    let mut lines = vec![format!("owners: {}", list.owners.len())];
    lines.extend(list.owners.iter().map(|owner| {
        let kinds: Vec<String> = owner
            .kinds
            .iter()
            .map(|kind| format!("{} {}", kind.kind, kind.count))
            .collect();
        format!(
            "  {}  {} memories ({}), next seq {}",
            owner.owner,
            owner.memories,
            kinds.join(", "),
            owner.next_seq
        )
    }));
    lines.join("\n")
}

fn format_page(page: &MemoryPage) -> String {
    let mut lines = vec![format!("memories: {}", page.memories.len())];
    lines.extend(page.memories.iter().map(|memory| {
        format!(
            "  {:>4}  {}  ({})  {}  place={}  pictures={}",
            memory.seq,
            memory.id,
            memory.kind,
            memory.title,
            memory.place.as_deref().unwrap_or("-"),
            comma(&memory.pictures)
        )
    }));
    if let Some(next) = &page.next {
        lines.push(format!("next: {next}"));
    }
    lines.join("\n")
}

fn format_references(references: &References) -> String {
    format!(
        "references: pictures={} memories={} left_out={}",
        comma(&references.pictures),
        comma(&references.memories),
        comma(&references.left_out)
    )
}

fn format_composed(composed: &Composed) -> String {
    format!(
        "ok picture={} members={} columns={} rows={}\nmembers: {}",
        composed.picture,
        composed.members.len(),
        composed.columns,
        composed.rows,
        comma(&composed.members)
    )
}

fn format_history(history: &History) -> String {
    if history.entries.is_empty() {
        return "no memory changes in the host's log".to_owned();
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

/// The query string of `pairs` that are given (`key=value&..`), after `?`.
fn query_of(pairs: &[(&str, Option<String>)]) -> String {
    let given: Vec<String> = pairs
        .iter()
        .filter_map(|(key, value)| value.as_ref().map(|value| format!("{key}={value}")))
        .collect();
    if given.is_empty() {
        String::new()
    } else {
        format!("?{}", given.join("&"))
    }
}

/// Runs one `world memory` command against the host on `port`.
pub async fn run(port: u16, command: MemoryCommand) -> Result<String, DevError> {
    match command {
        MemoryCommand::Owners => {
            let list: OwnerList = value(get_json(port, path::MEMORY).await?)?;
            Ok(format_owners(&list))
        }
        MemoryCommand::Put { file } => {
            let draft: MemoryDraft = file_json(&file).await?;
            let put: Put = value(post_json(port, path::MEMORY_PUT, &draft).await?)?;
            Ok(format_put(&put))
        }
        MemoryCommand::Get { owner, id } => {
            let address = format!("{}?owner={owner}&id={id}", path::MEMORY_GET);
            let memory: Memory = value(get_json(port, &address).await?)?;
            Ok(format_memory(&memory))
        }
        MemoryCommand::List {
            owner,
            filter,
            limit,
            after,
        } => {
            let query = query_of(&[
                ("owner", Some(owner)),
                ("kind", filter.kind),
                ("place", filter.place),
                ("tag", filter.tag),
                ("limit", limit.map(|limit| limit.to_string())),
                ("after", after),
            ]);
            let page: MemoryPage =
                value(get_json(port, &format!("{}{query}", path::MEMORY_LIST)).await?)?;
            Ok(format_page(&page))
        }
        MemoryCommand::Delete(request) => {
            let deleted: Deleted = value(post_json(port, path::MEMORY_DELETE, &request).await?)?;
            Ok(format!(
                "ok deleted {}\n{}",
                deleted.memory.id,
                format_memory(&deleted.memory)
            ))
        }
        MemoryCommand::Capture(request) => {
            let put: Put = value(post_json(port, path::MEMORY_CAPTURE, &request).await?)?;
            Ok(format_put(&put))
        }
        MemoryCommand::References(request) => {
            let references: References =
                value(post_json(port, path::MEMORY_REFERENCES, &request).await?)?;
            Ok(format_references(&references))
        }
        MemoryCommand::Compose(request) => {
            let composed: Composed = value(post_json(port, path::MEMORY_COMPOSE, &request).await?)?;
            Ok(format_composed(&composed))
        }
        MemoryCommand::History { owner, limit } => {
            let query = query_of(&[
                ("owner", owner),
                ("limit", limit.map(|limit| limit.to_string())),
            ]);
            let history: History =
                value(get_json(port, &format!("{}{query}", path::MEMORY_HISTORY)).await?)?;
            Ok(format_history(&history))
        }
    }
}
