//! The bodies and replies of the memory routes, and why they refuse.
//!
//! Every route answers a `MemoryReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`.

use std::fmt;

use axum::http::StatusCode;
use df_imagery::gallery::GalleryError;
use df_playground::kit::ledger::Ledger;
use serde::{Deserialize, Serialize};

use super::{Memory, MemoryField, MemoryFilter, MemoryLink};
use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::io_cause::IoCause;

/// The answer of a memory route: its value, or why it was refused.
pub type MemoryReply<T> = DevReply<T, MemoryError>;

/// `GET /dev/world/memory/get`: one memory of an owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetQuery {
    pub owner: String,
    pub id: String,
}

/// `GET /dev/world/memory/list`: the memories of an owner that the filter means, oldest first,
/// `limit` of them after the cursor `after` (the `next` of the page before).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub owner: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub after: Option<String>,
}

impl ListQuery {
    pub fn filter(&self) -> MemoryFilter {
        MemoryFilter {
            kind: self.kind.clone(),
            place: self.place.clone(),
            tag: self.tag.clone(),
        }
    }
}

/// `POST /dev/world/memory/delete`: the memory leaves the owner; its pictures stay in the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteRequest {
    pub owner: String,
    pub id: String,
}

/// `POST /dev/world/memory/capture`: a memory of one existing picture, in one call. The id is
/// `<kind>-<seq>` when none is given and the description is the title; a capture never replaces a
/// memory (an id that is kept is `duplicate_id`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRequest {
    pub owner: String,
    pub kind: String,
    pub picture: String,
    pub title: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub links: Vec<MemoryLink>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_by: Option<String>,
}

/// `POST /dev/world/memory/references`: the pictures of an owner's memories to attach to a plate,
/// at most `budget` of them. The memories are `ids` in the order given, or the ones `filter` means,
/// newest first; naming both is `conflicting_selection`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferencesRequest {
    pub owner: String,
    #[serde(default)]
    pub filter: MemoryFilter,
    #[serde(default)]
    pub ids: Vec<String>,
    pub budget: usize,
}

/// `POST /dev/world/memory/compose`: the primary pictures of the selected memories (`ids`, or what
/// `filter` means, every memory when it names nothing; both is `conflicting_selection`) tiled in
/// `seq` order into one gallery sheet, stored in the media library as `name` (`gallery-<owner>-<n>`
/// when none is given). The sheet is laid out as the fields say, and as
/// `df_imagery::gallery::GallerySpec::default` where they are left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComposeRequest {
    pub owner: String,
    #[serde(default)]
    pub filter: MemoryFilter,
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub columns: Option<u32>,
    #[serde(default)]
    pub tile_width_px: Option<u32>,
    #[serde(default)]
    pub tile_height_px: Option<u32>,
    #[serde(default)]
    pub gap_px: Option<u32>,
    /// The colour behind and between the tiles: `[red, green, blue]`.
    #[serde(default)]
    pub background: Option<[u8; 3]>,
}

/// The query of `GET /dev/world/memory/history`: the memory lines of one owner (all owners when
/// none), the newest `limit` of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// How many memories of a kind an owner keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindCount {
    pub kind: String,
    pub count: usize,
}

/// One owner in the index: how many memories it keeps, of which kinds, and the `seq` the next will
/// get.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerSummary {
    pub owner: String,
    pub memories: usize,
    pub next_seq: u64,
    pub kinds: Vec<KindCount>,
}

/// What `GET /dev/world/memory` answers: every owner, in id order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerList {
    pub owners: Vec<OwnerSummary>,
}

/// A page of memories, oldest first, `next` the cursor of the page after it when there is one, and
/// the same page as the rows of the kit's list panel for the page: one row for each memory with
/// its primary picture (`id`, `name` the title, `picture`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryPage {
    pub memories: Vec<Memory>,
    pub next: Option<String>,
    pub ledger: Ledger,
}

/// What a delete came to: the memory that left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deleted {
    pub memory: Memory,
}

/// What a compose came to: the gallery sheet now in the media library (`picture`, a normal picture
/// to attach as one reference), the memories it shows in `seq` order, their primary pictures in the
/// same order, and its grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Composed {
    pub owner: String,
    pub picture: String,
    pub members: Vec<String>,
    pub pictures: Vec<String>,
    pub columns: u32,
    pub rows: u32,
}

/// One line of the memory history: when the host logged it, and the line (ids only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub sequence: u64,
    pub at_ms: u64,
    pub line: String,
}

/// What `GET /dev/world/memory/history` answers, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

/// Why a memory route refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum MemoryError {
    InvalidBody(BodyError),
    InvalidQuery {
        detail: String,
    },
    /// An owner id is not an id.
    InvalidOwner {
        owner: String,
    },
    /// A memory id is not an id.
    InvalidId {
        id: String,
    },
    /// A kind is not an id.
    InvalidKind {
        kind: String,
    },
    /// A place is not an id.
    InvalidPlace {
        place: String,
    },
    /// A tag is not an id.
    InvalidTag {
        tag: String,
    },
    /// A link names an id that is not an id.
    InvalidLink {
        id: String,
    },
    /// A picture name is not a plain name of an image.
    InvalidPicture {
        picture: String,
    },
    /// A gallery name is not an id.
    InvalidName {
        name: String,
    },
    /// A cursor is not one a list gave.
    InvalidCursor,
    /// A stored or sent book has a schema this host does not read.
    InvalidSchema {
        schema: u32,
    },
    /// A stored book's `seq` does not rise, or is not below `next_seq`.
    InvalidSequence {
        id: String,
    },
    /// The owner keeps no memories (make one with `put` or `capture`).
    UnknownOwner {
        owner: String,
    },
    /// The owner keeps no such memory.
    UnknownMemory {
        owner: String,
        id: String,
    },
    /// A picture is not in the media library.
    UnknownPicture {
        picture: String,
    },
    /// A put is for another owner than the book it is put into.
    OwnerMismatch {
        owner: String,
        book: String,
    },
    /// A capture's id is already kept by the owner.
    DuplicateId {
        id: String,
    },
    DuplicatePicture {
        picture: String,
    },
    DuplicateTag {
        tag: String,
    },
    DuplicateLink {
        id: String,
    },
    /// A selection names a memory twice.
    DuplicateSelected {
        id: String,
    },
    /// A selection names both ids and a filter.
    ConflictingSelection,
    EmptyTitle {
        id: String,
    },
    EmptyDescription {
        id: String,
    },
    /// A text is longer than its bound.
    TooLong {
        field: MemoryField,
        maximum: usize,
    },
    /// A count passes its bound.
    TooMany {
        field: MemoryField,
        maximum: usize,
    },
    /// The selection has no memory with a picture to tile.
    NothingToCompose,
    /// The gallery sheet could not be laid out.
    Gallery {
        cause: GalleryError,
    },
    /// A picture of the library could not be read.
    PictureUnreadable {
        picture: String,
        cause: IoCause,
    },
    /// The gallery sheet could not be stored in the media library.
    PictureNotStored {
        detail: String,
    },
    /// A memory file could not be read.
    StoreUnreadable {
        cause: IoCause,
    },
    /// A memory file is not the stored shape.
    StoreInvalid {
        cause: BodyError,
    },
    /// A memory file could not be written (nothing was changed).
    StoreUnwritable {
        cause: IoCause,
    },
    Stopped,
}

impl MemoryError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::InvalidQuery { .. } => "invalid_query",
            Self::InvalidOwner { .. } => "invalid_owner",
            Self::InvalidId { .. } => "invalid_id",
            Self::InvalidKind { .. } => "invalid_kind",
            Self::InvalidPlace { .. } => "invalid_place",
            Self::InvalidTag { .. } => "invalid_tag",
            Self::InvalidLink { .. } => "invalid_link",
            Self::InvalidPicture { .. } => "invalid_picture",
            Self::InvalidName { .. } => "invalid_name",
            Self::InvalidCursor => "invalid_cursor",
            Self::InvalidSchema { .. } => "invalid_schema",
            Self::InvalidSequence { .. } => "invalid_sequence",
            Self::UnknownOwner { .. } => "unknown_owner",
            Self::UnknownMemory { .. } => "unknown_memory",
            Self::UnknownPicture { .. } => "unknown_picture",
            Self::OwnerMismatch { .. } => "owner_mismatch",
            Self::DuplicateId { .. } => "duplicate_id",
            Self::DuplicatePicture { .. } => "duplicate_picture",
            Self::DuplicateTag { .. } => "duplicate_tag",
            Self::DuplicateLink { .. } => "duplicate_link",
            Self::DuplicateSelected { .. } => "duplicate_selected",
            Self::ConflictingSelection => "conflicting_selection",
            Self::EmptyTitle { .. } => "empty_title",
            Self::EmptyDescription { .. } => "empty_description",
            Self::TooLong { .. } => "too_long",
            Self::TooMany { .. } => "too_many",
            Self::NothingToCompose => "nothing_to_compose",
            Self::Gallery { .. } => "gallery",
            Self::PictureUnreadable { .. } => "picture_unreadable",
            Self::PictureNotStored { .. } => "picture_not_stored",
            Self::StoreUnreadable { .. } => "store_unreadable",
            Self::StoreInvalid { .. } => "store_invalid",
            Self::StoreUnwritable { .. } => "store_unwritable",
            Self::Stopped => "stopped",
        }
    }
}

impl From<BodyError> for MemoryError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        const ID_RULE: &str = "lowercase letters, digits, '_' or '-'";
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::InvalidQuery { detail } => write!(formatter, "the query is not valid: {detail}"),
            Self::InvalidOwner { owner } => {
                write!(formatter, "{owner:?} is not an owner id ({ID_RULE})")
            }
            Self::InvalidId { id } => write!(formatter, "{id:?} is not a memory id ({ID_RULE})"),
            Self::InvalidKind { kind } => write!(formatter, "{kind:?} is not a kind ({ID_RULE})"),
            Self::InvalidPlace { place } => {
                write!(formatter, "{place:?} is not a place id ({ID_RULE})")
            }
            Self::InvalidTag { tag } => write!(formatter, "{tag:?} is not a tag ({ID_RULE})"),
            Self::InvalidLink { id } => {
                write!(formatter, "a link names {id:?}, which is not an id")
            }
            Self::InvalidPicture { picture } => {
                write!(formatter, "{picture:?} is not a plain name of a picture")
            }
            Self::InvalidName { name } => {
                write!(formatter, "{name:?} is not a gallery name ({ID_RULE})")
            }
            Self::InvalidCursor => formatter.write_str("the cursor is not one a list gave"),
            Self::InvalidSchema { schema } => {
                write!(
                    formatter,
                    "memory schema {schema} is not one this host reads"
                )
            }
            Self::InvalidSequence { id } => {
                write!(
                    formatter,
                    "the stored order of {id} is not a rising sequence"
                )
            }
            Self::UnknownOwner { owner } => {
                write!(
                    formatter,
                    "{owner} keeps no memories (see: world memory put)"
                )
            }
            Self::UnknownMemory { owner, id } => write!(formatter, "{owner} keeps no memory {id}"),
            Self::UnknownPicture { picture } => {
                write!(formatter, "the media library has no picture {picture}")
            }
            Self::OwnerMismatch { owner, book } => {
                write!(
                    formatter,
                    "a memory of {owner} cannot go into the memories of {book}"
                )
            }
            Self::DuplicateId { id } => write!(formatter, "the owner already keeps a memory {id}"),
            Self::DuplicatePicture { picture } => {
                write!(formatter, "{picture} is named twice by one memory")
            }
            Self::DuplicateTag { tag } => write!(formatter, "the tag {tag} is named twice"),
            Self::DuplicateLink { id } => write!(formatter, "the link to {id} is named twice"),
            Self::DuplicateSelected { id } => write!(formatter, "{id} is selected twice"),
            Self::ConflictingSelection => {
                formatter.write_str("a selection is ids or a filter, not both")
            }
            Self::EmptyTitle { id } => write!(formatter, "{id} needs a title that says something"),
            Self::EmptyDescription { id } => {
                write!(formatter, "{id} needs a description that says something")
            }
            Self::TooLong { field, maximum } => {
                write!(formatter, "the {field} is longer than {maximum} characters")
            }
            Self::TooMany { field, maximum } => {
                write!(
                    formatter,
                    "there are more {field} than the {maximum} allowed"
                )
            }
            Self::NothingToCompose => {
                formatter.write_str("no selected memory has a picture to tile")
            }
            Self::Gallery { cause } => write!(formatter, "the gallery sheet: {cause}"),
            Self::PictureUnreadable { picture, cause } => {
                write!(
                    formatter,
                    "the picture {picture} could not be read: {cause}"
                )
            }
            Self::PictureNotStored { detail } => {
                write!(formatter, "the gallery sheet could not be stored: {detail}")
            }
            Self::StoreUnreadable { cause } => {
                write!(formatter, "a memory file could not be read: {cause}")
            }
            Self::StoreInvalid { cause } => write!(formatter, "a memory file: {cause}"),
            Self::StoreUnwritable { cause } => {
                write!(formatter, "a memory file could not be written: {cause}")
            }
            Self::Stopped => formatter.write_str("the host stopped before the work finished"),
        }
    }
}

impl std::error::Error for MemoryError {}

impl RouteError for MemoryError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::InvalidQuery { .. }
            | Self::InvalidOwner { .. }
            | Self::InvalidId { .. }
            | Self::InvalidKind { .. }
            | Self::InvalidPlace { .. }
            | Self::InvalidTag { .. }
            | Self::InvalidLink { .. }
            | Self::InvalidPicture { .. }
            | Self::InvalidName { .. }
            | Self::InvalidCursor
            | Self::InvalidSchema { .. }
            | Self::OwnerMismatch { .. }
            | Self::DuplicatePicture { .. }
            | Self::DuplicateTag { .. }
            | Self::DuplicateLink { .. }
            | Self::DuplicateSelected { .. }
            | Self::ConflictingSelection
            | Self::EmptyTitle { .. }
            | Self::EmptyDescription { .. }
            | Self::TooLong { .. }
            | Self::TooMany { .. } => StatusCode::BAD_REQUEST,
            Self::UnknownOwner { .. }
            | Self::UnknownMemory { .. }
            | Self::UnknownPicture { .. } => StatusCode::NOT_FOUND,
            Self::DuplicateId { .. } | Self::NothingToCompose | Self::Gallery { .. } => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            Self::InvalidSequence { .. }
            | Self::PictureUnreadable { .. }
            | Self::PictureNotStored { .. }
            | Self::StoreUnreadable { .. }
            | Self::StoreInvalid { .. }
            | Self::StoreUnwritable { .. }
            | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
