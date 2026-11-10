//! The standard memory of the dev host: what any owner keeps as records with pictures, for any
//! number of owners and records. A memory is a scene visited, a picture painted, a note with a
//! sketch: the host does not tell them apart. Its `kind` is an id-grammar string chosen by the
//! story or the runner (`scene`, `artwork`, `note`, ..), data and never a type of this code. The
//! host keeps one file per owner and an index of owners (`store`), reads them again on every call
//! and writes them whole or not at all, as the inventory does.
//!
//! The contract is typed and keyed by an owner id, the inventory's owner space
//! (`df_content::world_bible::is_valid_id`, the id grammar of the world bible). A memory is
//! `{id, owner, kind, title, description, place, seq, pictures, links, tags, created_by}`:
//!
//! - `id` is unique within its owner. A put of an id the owner has replaces it, keeping its `seq`.
//! - `description` is the look in words, so the picture can be redrawn or told without its file.
//! - `seq` is assigned by code, per owner, only ever up: the order things happened, never reused,
//!   even after a delete.
//! - `pictures` are names of the media library (the first is the primary); every one must be there
//!   when the memory is put. Deleting a memory never deletes a picture.
//! - `links` are typed (`item`, `memory`, `place`); a `memory` link names a memory the same owner
//!   keeps, `item` and `place` links are ids in the grammar. They are not checked against the
//!   inventory or the bible: a place may be designed in play.
//!
//! A memory is a record that something happened and how it looked. It is never a rule and never
//! the canonical truth of the world (`planning/long-horizon-state.md`): a summary or an index of
//! the world does not replace it, and nothing in a memory is executed.
//!
//! The pure rules are [`check_draft`], [`select`], [`page`] and [`references_for`]; `operations`
//! applies them to the store and `routes` answers them. `compose` tiles the primary pictures of a
//! selection into one gallery sheet (`df_imagery::gallery`), so a collection too large to attach
//! picture by picture is attached as one.
//!
//! Where this belongs later: `df_model` / `df-knowledge` own `MemoryEpisode` and its retrieval
//! (committed events with audience scope); this is the dev host's record of pictures, and its owner
//! and memory ids carry over.

pub mod cli;
mod host;
pub mod limits;
mod operations;
pub mod request;
pub mod routes;
mod store;

use std::fmt;

use df_content::world_bible::is_valid_id;
use df_playground::media::{MediaKind, media_kind};
use serde::{Deserialize, Serialize};

pub(crate) use self::host::{keep, recent};
use self::limits::{
    MAXIMUM_CREATED_BY_CHARS, MAXIMUM_DESCRIPTION_CHARS, MAXIMUM_LINKS, MAXIMUM_PICTURES,
    MAXIMUM_TAGS, MAXIMUM_TITLE_CHARS,
};
pub use self::request::MemoryError;

/// The version of the stored shape, written in every owner file and the index.
pub const SCHEMA: u32 = 1;

/// What a link of a memory points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// An item of the inventory.
    Item,
    /// Another memory of the same owner.
    Memory,
    /// A place.
    Place,
}

impl LinkKind {
    pub const ALL: [Self; 3] = [Self::Item, Self::Memory, Self::Place];

    pub fn key(self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Memory => "memory",
            Self::Place => "place",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// A typed link from a memory to an item, another memory or a place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryLink {
    pub what: LinkKind,
    pub id: String,
}

/// One kept record with pictures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    pub id: String,
    pub owner: String,
    pub kind: String,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    pub seq: u64,
    pub pictures: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<MemoryLink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
}

impl Memory {
    /// The first picture, the one a plate or a sheet shows the memory by.
    pub fn primary(&self) -> Option<&str> {
        self.pictures.first().map(String::as_str)
    }
}

/// A memory as it is put: everything but `seq`, which code assigns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryDraft {
    pub id: String,
    pub owner: String,
    pub kind: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub pictures: Vec<String>,
    #[serde(default)]
    pub links: Vec<MemoryLink>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_by: Option<String>,
}

/// Everything one owner keeps, in `seq` order. `next_seq` is the `seq` of the next memory and
/// `galleries` counts the gallery sheets numbered for the owner, both only ever up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryBook {
    pub schema: u32,
    pub owner: String,
    pub next_seq: u64,
    pub galleries: u64,
    pub memories: Vec<Memory>,
}

/// Which of an owner's memories are meant; a field left out matches any.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryFilter {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
}

impl MemoryFilter {
    /// Whether `memory` is one the filter means.
    pub fn matches(&self, memory: &Memory) -> bool {
        self.kind.as_ref().is_none_or(|kind| *kind == memory.kind)
            && self
                .place
                .as_ref()
                .is_none_or(|place| memory.place.as_ref() == Some(place))
            && self
                .tag
                .as_ref()
                .is_none_or(|tag| memory.tags.contains(tag))
    }

    /// Whether the filter names nothing, so it means every memory.
    pub fn is_empty(&self) -> bool {
        self.kind.is_none() && self.place.is_none() && self.tag.is_none()
    }

    /// Checks the ids the filter names are in the grammar.
    pub fn check(&self) -> Result<(), MemoryError> {
        if let Some(kind) = &self.kind {
            check_kind(kind)?;
        }
        if let Some(place) = &self.place {
            check_place(place)?;
        }
        if let Some(tag) = &self.tag {
            check_tag(tag)?;
        }
        Ok(())
    }
}

/// The limit a count or a text passes, named for the refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryField {
    Title,
    Description,
    CreatedBy,
    Pictures,
    Links,
    Tags,
    Memories,
}

impl MemoryField {
    pub const ALL: [Self; 7] = [
        Self::Title,
        Self::Description,
        Self::CreatedBy,
        Self::Pictures,
        Self::Links,
        Self::Tags,
        Self::Memories,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Description => "description",
            Self::CreatedBy => "created_by",
            Self::Pictures => "pictures",
            Self::Links => "links",
            Self::Tags => "tags",
            Self::Memories => "memories",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.key() == key)
    }
}

impl fmt::Display for MemoryField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.key())
    }
}

pub(super) fn check_owner(owner: &str) -> Result<(), MemoryError> {
    if is_valid_id(owner) {
        Ok(())
    } else {
        Err(MemoryError::InvalidOwner {
            owner: owner.to_owned(),
        })
    }
}

pub(super) fn check_memory_id(id: &str) -> Result<(), MemoryError> {
    if is_valid_id(id) {
        Ok(())
    } else {
        Err(MemoryError::InvalidId { id: id.to_owned() })
    }
}

pub(super) fn check_kind(kind: &str) -> Result<(), MemoryError> {
    if is_valid_id(kind) {
        Ok(())
    } else {
        Err(MemoryError::InvalidKind {
            kind: kind.to_owned(),
        })
    }
}

fn check_place(place: &str) -> Result<(), MemoryError> {
    if is_valid_id(place) {
        Ok(())
    } else {
        Err(MemoryError::InvalidPlace {
            place: place.to_owned(),
        })
    }
}

fn check_tag(tag: &str) -> Result<(), MemoryError> {
    if is_valid_id(tag) {
        Ok(())
    } else {
        Err(MemoryError::InvalidTag {
            tag: tag.to_owned(),
        })
    }
}

/// A picture name that is a plain name of an image of the media library.
pub(super) fn check_picture_name(picture: &str) -> Result<(), MemoryError> {
    if media_kind(picture) == Some(MediaKind::Image) {
        Ok(())
    } else {
        Err(MemoryError::InvalidPicture {
            picture: picture.to_owned(),
        })
    }
}

fn check_text(text: &str, field: MemoryField, maximum: usize, id: &str) -> Result<(), MemoryError> {
    if text.trim().is_empty() {
        return Err(match field {
            MemoryField::Title => MemoryError::EmptyTitle { id: id.to_owned() },
            _ => MemoryError::EmptyDescription { id: id.to_owned() },
        });
    }
    if text.chars().count() > maximum {
        return Err(MemoryError::TooLong { field, maximum });
    }
    Ok(())
}

fn check_count(count: usize, field: MemoryField, maximum: usize) -> Result<(), MemoryError> {
    if count > maximum {
        return Err(MemoryError::TooMany { field, maximum });
    }
    Ok(())
}

/// Checks everything of `draft` that needs no store: ids in the grammar, a title and a description
/// that say something, bounds, no picture, tag or link twice, and picture names that are plain
/// image names. Whether the pictures exist and the linked memories are kept is the store's to
/// check.
pub fn check_draft(draft: &MemoryDraft) -> Result<(), MemoryError> {
    check_owner(&draft.owner)?;
    check_memory_id(&draft.id)?;
    check_kind(&draft.kind)?;
    if let Some(place) = &draft.place {
        check_place(place)?;
    }
    check_text(
        &draft.title,
        MemoryField::Title,
        MAXIMUM_TITLE_CHARS,
        &draft.id,
    )?;
    check_text(
        &draft.description,
        MemoryField::Description,
        MAXIMUM_DESCRIPTION_CHARS,
        &draft.id,
    )?;
    if let Some(created_by) = &draft.created_by
        && created_by.chars().count() > MAXIMUM_CREATED_BY_CHARS
    {
        return Err(MemoryError::TooLong {
            field: MemoryField::CreatedBy,
            maximum: MAXIMUM_CREATED_BY_CHARS,
        });
    }
    check_count(
        draft.pictures.len(),
        MemoryField::Pictures,
        MAXIMUM_PICTURES,
    )?;
    check_count(draft.links.len(), MemoryField::Links, MAXIMUM_LINKS)?;
    check_count(draft.tags.len(), MemoryField::Tags, MAXIMUM_TAGS)?;
    for (index, picture) in draft.pictures.iter().enumerate() {
        check_picture_name(picture)?;
        if draft.pictures[..index].contains(picture) {
            return Err(MemoryError::DuplicatePicture {
                picture: picture.clone(),
            });
        }
    }
    for (index, tag) in draft.tags.iter().enumerate() {
        check_tag(tag)?;
        if draft.tags[..index].contains(tag) {
            return Err(MemoryError::DuplicateTag { tag: tag.clone() });
        }
    }
    for (index, link) in draft.links.iter().enumerate() {
        if !is_valid_id(&link.id) {
            return Err(MemoryError::InvalidLink {
                id: link.id.clone(),
            });
        }
        if draft.links[..index].contains(link) {
            return Err(MemoryError::DuplicateLink {
                id: link.id.clone(),
            });
        }
    }
    Ok(())
}

impl MemoryBook {
    /// A book with no memories yet.
    pub fn new(owner: &str) -> Result<Self, MemoryError> {
        check_owner(owner)?;
        Ok(Self {
            schema: SCHEMA,
            owner: owner.to_owned(),
            next_seq: 1,
            galleries: 0,
            memories: Vec::new(),
        })
    }

    /// Checks a stored book: the schema, the owner on every memory, ids in the grammar and not
    /// twice, `seq` strictly rising and below `next_seq`.
    pub fn check(&self) -> Result<(), MemoryError> {
        if self.schema != SCHEMA {
            return Err(MemoryError::InvalidSchema {
                schema: self.schema,
            });
        }
        check_owner(&self.owner)?;
        let mut last = 0;
        for (index, memory) in self.memories.iter().enumerate() {
            check_memory_id(&memory.id)?;
            if memory.owner != self.owner {
                return Err(MemoryError::InvalidOwner {
                    owner: memory.owner.clone(),
                });
            }
            if memory.seq <= last || memory.seq >= self.next_seq {
                return Err(MemoryError::InvalidSequence {
                    id: memory.id.clone(),
                });
            }
            last = memory.seq;
            if self.memories[..index]
                .iter()
                .any(|seen| seen.id == memory.id)
            {
                return Err(MemoryError::InvalidId {
                    id: memory.id.clone(),
                });
            }
        }
        Ok(())
    }

    /// The memory `id`, if the owner keeps it.
    pub fn memory(&self, id: &str) -> Option<&Memory> {
        self.memories.iter().find(|memory| memory.id == id)
    }

    /// The kinds the owner keeps, with how many of each, in id order.
    pub fn kind_counts(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for memory in &self.memories {
            match counts.iter_mut().find(|(kind, _)| *kind == memory.kind) {
                Some((_, count)) => *count += 1,
                None => counts.push((memory.kind.clone(), 1)),
            }
        }
        counts.sort();
        counts
    }
}

/// What a put came to: the memory as kept, and whether it is new.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Put {
    pub memory: Memory,
    pub created: bool,
}

/// `draft` put into `book`: a new memory with the next `seq`, or the replacement of the memory of
/// the same id with its `seq` kept. Refused whole, with nothing changed, when the draft is for
/// another owner than the book's, a `memory` link names a memory the owner does not keep (a memory
/// just put may not link to itself unless it is replacing one already kept), or the owner keeps as
/// many as the bound.
pub fn put_into(book: &mut MemoryBook, draft: MemoryDraft) -> Result<Put, MemoryError> {
    check_draft(&draft)?;
    if draft.owner != book.owner {
        return Err(MemoryError::OwnerMismatch {
            owner: draft.owner,
            book: book.owner.clone(),
        });
    }
    for link in draft
        .links
        .iter()
        .filter(|link| link.what == LinkKind::Memory)
    {
        if book.memory(&link.id).is_none() {
            return Err(MemoryError::UnknownMemory {
                owner: book.owner.clone(),
                id: link.id.clone(),
            });
        }
    }
    let existing = book
        .memories
        .iter()
        .position(|memory| memory.id == draft.id);
    if existing.is_none() && book.memories.len() >= limits::MAXIMUM_MEMORIES_PER_OWNER {
        return Err(MemoryError::TooMany {
            field: MemoryField::Memories,
            maximum: limits::MAXIMUM_MEMORIES_PER_OWNER,
        });
    }
    let seq = match existing {
        Some(index) => book.memories[index].seq,
        None => {
            let seq = book.next_seq;
            book.next_seq = seq.checked_add(1).ok_or(MemoryError::TooMany {
                field: MemoryField::Memories,
                maximum: limits::MAXIMUM_MEMORIES_PER_OWNER,
            })?;
            seq
        }
    };
    let memory = Memory {
        id: draft.id,
        owner: draft.owner,
        kind: draft.kind,
        title: draft.title,
        description: draft.description,
        place: draft.place,
        seq,
        pictures: draft.pictures,
        links: draft.links,
        tags: draft.tags,
        created_by: draft.created_by,
    };
    match existing {
        Some(index) => book.memories[index] = memory.clone(),
        None => book.memories.push(memory.clone()),
    }
    Ok(Put {
        memory,
        created: existing.is_none(),
    })
}

/// The memories of `book` that `filter` means, oldest first.
pub fn select<'a>(book: &'a MemoryBook, filter: &MemoryFilter) -> Vec<&'a Memory> {
    book.memories
        .iter()
        .filter(|memory| filter.matches(memory))
        .collect()
}

/// The cursor that continues a list after the memory of `seq`. Callers pass it back unread.
pub fn cursor_after(seq: u64) -> String {
    format!("c{seq:x}")
}

fn seq_of_cursor(cursor: &str) -> Result<u64, MemoryError> {
    cursor
        .strip_prefix('c')
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|digits| u64::from_str_radix(digits, 16).ok())
        .ok_or(MemoryError::InvalidCursor)
}

/// One page of the memories `filter` means, oldest first: after the cursor `after` when given, at
/// most `limit`. The cursor of the next page is given when there are more.
pub fn page<'a>(
    book: &'a MemoryBook,
    filter: &MemoryFilter,
    limit: usize,
    after: Option<&str>,
) -> Result<(Vec<&'a Memory>, Option<String>), MemoryError> {
    let after = after.map(seq_of_cursor).transpose()?.unwrap_or(0);
    let mut rest = book
        .memories
        .iter()
        .filter(|memory| memory.seq > after && filter.matches(memory));
    let page: Vec<&Memory> = rest.by_ref().take(limit).collect();
    let next = if rest.next().is_some() {
        page.last().map(|last| cursor_after(last.seq))
    } else {
        None
    };
    Ok((page, next))
}

/// The ids a request names, each once and kept by the owner, in the order given.
pub fn named<'a>(book: &'a MemoryBook, ids: &[String]) -> Result<Vec<&'a Memory>, MemoryError> {
    let mut found: Vec<&Memory> = Vec::with_capacity(ids.len());
    for id in ids {
        check_memory_id(id)?;
        let memory = book.memory(id).ok_or_else(|| MemoryError::UnknownMemory {
            owner: book.owner.clone(),
            id: id.clone(),
        })?;
        if found.iter().any(|seen| seen.id == memory.id) {
            return Err(MemoryError::DuplicateSelected { id: id.clone() });
        }
        found.push(memory);
    }
    Ok(found)
}

/// The pictures to attach to a plate for a selection, within a budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct References {
    /// The primary pictures that fit the budget, in the order they should be attached.
    pub pictures: Vec<String>,
    /// The memories those pictures stand for, in the same order.
    pub memories: Vec<String>,
    /// The primary pictures that did not fit, in order: write them down, never send them.
    pub left_out: Vec<String>,
}

/// The primary pictures of `chosen`, in order, the first `budget` of them (a picture twice is one):
/// what fits is attached, what does not is `left_out`. A memory with no picture stands for none.
pub fn references_for(chosen: &[&Memory], budget: usize) -> References {
    let mut pictures: Vec<String> = Vec::new();
    let mut memories: Vec<String> = Vec::new();
    let mut left_out: Vec<String> = Vec::new();
    for memory in chosen {
        let Some(picture) = memory.primary() else {
            continue;
        };
        if pictures.iter().chain(&left_out).any(|seen| seen == picture) {
            continue;
        }
        if pictures.len() < budget {
            pictures.push(picture.to_owned());
            memories.push(memory.id.clone());
        } else {
            left_out.push(picture.to_owned());
        }
    }
    References {
        pictures,
        memories,
        left_out,
    }
}

/// The id a captured memory gets when none is given: its kind and its `seq`.
pub fn derived_id(kind: &str, seq: u64) -> String {
    format!("{kind}-{seq}")
}
