//! What each memory operation does to the store, with no HTTP and no logging: the routes call
//! these, so there is one validation and one way to change a memory.

use std::path::{Path, PathBuf};

use df_content::world_bible::is_valid_id;
use df_imagery::gallery::{GallerySpec, compose_gallery, gallery_grid};
use df_playground::kit::ledger::{Ledger, LedgerItem, LedgerKind};

use super::limits::{DEFAULT_PAGE, MAXIMUM_PAGE};
use super::request::{
    CaptureRequest, ComposeRequest, Deleted, GetQuery, History, HistoryEntry, HistoryQuery,
    ListQuery, MemoryError, MemoryPage, OwnerList, ReferencesRequest,
};
use super::store::{GallerySidecar, read_book, read_index, transact, write_sidecar};
use super::{
    Memory, MemoryBook, MemoryDraft, MemoryFilter, Put, References, SCHEMA, check_draft,
    check_kind, check_memory_id, check_owner, check_picture_name, derived_id, named, page,
    put_into, references_for, select,
};
use crate::event_log::EventLog;
use crate::io_cause::IoCause;

/// Where the memories and the pictures they name live.
pub(super) struct Paths {
    pub memory: PathBuf,
    pub media: PathBuf,
}

/// The rows of the kit's list panel for `memories`: one for each, titled, with its primary picture.
pub(super) fn ledger_of(memories: &[Memory]) -> Ledger {
    Ledger {
        title: None,
        icon: None,
        kind: LedgerKind::Loot,
        items: memories
            .iter()
            .map(|memory| LedgerItem {
                id: memory.id.clone(),
                name: memory.title.clone(),
                quantity: None,
                price: None,
                picture: memory.primary().map(str::to_owned),
                icon: None,
                rarity: None,
                taken: false,
                highlight: false,
            })
            .collect(),
        framed: true,
    }
}

/// Every owner, from the index.
pub(super) fn list_owners(directory: &Path) -> Result<OwnerList, MemoryError> {
    Ok(OwnerList {
        owners: read_index(directory)?,
    })
}

/// The memory `query.id` of `query.owner`.
pub(super) fn get(directory: &Path, query: &GetQuery) -> Result<Memory, MemoryError> {
    check_owner(&query.owner)?;
    check_memory_id(&query.id)?;
    let book = read_book(directory, &query.owner)?;
    book.memory(&query.id)
        .cloned()
        .ok_or_else(|| MemoryError::UnknownMemory {
            owner: query.owner.clone(),
            id: query.id.clone(),
        })
}

/// A page of the memories of `query.owner` that the query's filter means.
pub(super) fn list(directory: &Path, query: &ListQuery) -> Result<MemoryPage, MemoryError> {
    check_owner(&query.owner)?;
    let filter = query.filter();
    filter.check()?;
    let limit = query.limit.unwrap_or(DEFAULT_PAGE);
    if limit == 0 || limit > MAXIMUM_PAGE {
        return Err(MemoryError::InvalidQuery {
            detail: format!("limit must be from 1 to {MAXIMUM_PAGE}"),
        });
    }
    let book = read_book(directory, &query.owner)?;
    let (found, next) = page(&book, &filter, limit, query.after.as_deref())?;
    let memories: Vec<Memory> = found.into_iter().cloned().collect();
    Ok(MemoryPage {
        ledger: ledger_of(&memories),
        memories,
        next,
    })
}

/// Checks every picture of `pictures` is in the media library.
fn check_in_library(media: &Path, pictures: &[String]) -> Result<(), MemoryError> {
    for picture in pictures {
        if !media.join(picture).is_file() {
            return Err(MemoryError::UnknownPicture {
                picture: picture.clone(),
            });
        }
    }
    Ok(())
}

/// Puts `draft` into the memories of its owner (made when it is new).
pub(super) fn put(paths: &Paths, draft: MemoryDraft) -> Result<Put, MemoryError> {
    check_draft(&draft)?;
    check_in_library(&paths.media, &draft.pictures)?;
    let owner = draft.owner.clone();
    transact(&paths.memory, &owner, |stored| {
        let mut book = match stored {
            Some(book) => book,
            None => MemoryBook::new(&draft.owner)?,
        };
        let put = put_into(&mut book, draft)?;
        Ok((book, put))
    })
}

/// A memory of one existing picture: the next `seq` gives its id when none is asked for, its title
/// its description. Refused as `duplicate_id` when the id is kept.
pub(super) fn capture(paths: &Paths, request: CaptureRequest) -> Result<Put, MemoryError> {
    check_owner(&request.owner)?;
    check_kind(&request.kind)?;
    check_picture_name(&request.picture)?;
    check_in_library(&paths.media, std::slice::from_ref(&request.picture))?;
    let owner = request.owner.clone();
    transact(&paths.memory, &owner, |stored| {
        let mut book = match stored {
            Some(book) => book,
            None => MemoryBook::new(&request.owner)?,
        };
        let id = request
            .id
            .clone()
            .unwrap_or_else(|| derived_id(&request.kind, book.next_seq));
        if book.memory(&id).is_some() {
            return Err(MemoryError::DuplicateId { id });
        }
        let draft = MemoryDraft {
            id,
            owner: request.owner.clone(),
            kind: request.kind.clone(),
            description: request
                .description
                .clone()
                .unwrap_or_else(|| request.title.clone()),
            title: request.title.clone(),
            place: request.place.clone(),
            pictures: vec![request.picture.clone()],
            links: request.links.clone(),
            tags: request.tags.clone(),
            created_by: request.created_by.clone(),
        };
        let put = put_into(&mut book, draft)?;
        Ok((book, put))
    })
}

/// The memory `id` leaves `owner`. Its `seq` is not given again, and its pictures stay in the media
/// library.
pub(super) fn delete(directory: &Path, owner: &str, id: &str) -> Result<Deleted, MemoryError> {
    check_owner(owner)?;
    check_memory_id(id)?;
    transact(directory, owner, |stored| {
        let mut book = stored.ok_or_else(|| MemoryError::UnknownOwner {
            owner: owner.to_owned(),
        })?;
        let index = book
            .memories
            .iter()
            .position(|memory| memory.id == id)
            .ok_or_else(|| MemoryError::UnknownMemory {
                owner: owner.to_owned(),
                id: id.to_owned(),
            })?;
        let memory = book.memories.remove(index);
        Ok((book, Deleted { memory }))
    })
}

/// The memories a request selects from `book`: `ids` in the order given, or what `filter` means
/// (every memory when it names nothing), oldest first.
fn selected<'a>(
    book: &'a MemoryBook,
    filter: &MemoryFilter,
    ids: &[String],
) -> Result<Vec<&'a Memory>, MemoryError> {
    if ids.is_empty() {
        filter.check()?;
        Ok(select(book, filter))
    } else if filter.is_empty() {
        named(book, ids)
    } else {
        Err(MemoryError::ConflictingSelection)
    }
}

/// The pictures to attach for a selection, newest first when it is a filter and in the order given
/// when it is ids, within `request.budget`.
pub(super) fn references(
    directory: &Path,
    request: &ReferencesRequest,
) -> Result<References, MemoryError> {
    check_owner(&request.owner)?;
    let book = read_book(directory, &request.owner)?;
    let mut chosen = selected(&book, &request.filter, &request.ids)?;
    if request.ids.is_empty() {
        chosen.reverse();
    }
    Ok(references_for(&chosen, request.budget))
}

/// A gallery sheet made but not yet in the media library.
pub(super) struct Prepared {
    pub owner: String,
    pub name: String,
    pub members: Vec<String>,
    pub pictures: Vec<String>,
    pub columns: u32,
    pub rows: u32,
    pub sheet: Vec<u8>,
}

fn spec_of(request: &ComposeRequest) -> GallerySpec {
    let default = GallerySpec::default();
    GallerySpec {
        columns: request.columns,
        tile_width_px: request.tile_width_px.unwrap_or(default.tile_width_px),
        tile_height_px: request.tile_height_px.unwrap_or(default.tile_height_px),
        gap_px: request.gap_px.unwrap_or(default.gap_px),
        background: request.background.unwrap_or(default.background),
    }
}

/// Tiles the primary pictures of the selected memories, in `seq` order, into one sheet, and names
/// it `request.name` or `gallery-<owner>-<n>` with `n` the next number of the owner's galleries
/// (the number is taken only once the sheet is made). Memories with no picture are left out of it.
pub(super) fn prepare_compose(
    paths: &Paths,
    request: &ComposeRequest,
) -> Result<Prepared, MemoryError> {
    check_owner(&request.owner)?;
    if let Some(name) = &request.name
        && !is_valid_id(name)
    {
        return Err(MemoryError::InvalidName { name: name.clone() });
    }
    let book = read_book(&paths.memory, &request.owner)?;
    let mut chosen = selected(&book, &request.filter, &request.ids)?;
    chosen.sort_by_key(|memory| memory.seq);
    let shown: Vec<(&Memory, &str)> = chosen
        .into_iter()
        .filter_map(|memory| memory.primary().map(|picture| (memory, picture)))
        .collect();
    if shown.is_empty() {
        return Err(MemoryError::NothingToCompose);
    }
    let spec = spec_of(request);
    let (columns, rows) =
        gallery_grid(shown.len(), &spec).map_err(|cause| MemoryError::Gallery { cause })?;
    let mut bytes: Vec<Vec<u8>> = Vec::with_capacity(shown.len());
    for (_, picture) in &shown {
        let read = std::fs::read(paths.media.join(picture)).map_err(|error| {
            MemoryError::PictureUnreadable {
                picture: (*picture).to_owned(),
                cause: IoCause::from_kind(error.kind()),
            }
        })?;
        bytes.push(read);
    }
    let sheet = compose_gallery(&bytes, &spec).map_err(|cause| MemoryError::Gallery { cause })?;
    let name = match &request.name {
        Some(name) => name.clone(),
        None => {
            let number = transact(&paths.memory, &request.owner, |stored| {
                let mut book = stored.ok_or_else(|| MemoryError::UnknownOwner {
                    owner: request.owner.clone(),
                })?;
                book.galleries = book.galleries.saturating_add(1);
                let number = book.galleries;
                Ok((book, number))
            })?;
            format!("gallery-{}-{number}", request.owner)
        }
    };
    Ok(Prepared {
        owner: request.owner.clone(),
        name,
        members: shown.iter().map(|(memory, _)| memory.id.clone()).collect(),
        pictures: shown
            .iter()
            .map(|(_, picture)| (*picture).to_owned())
            .collect(),
        columns,
        rows,
        sheet,
    })
}

/// Writes the sidecar that lists the members of the sheet `prepared` made.
pub(super) fn keep_members(directory: &Path, prepared: &Prepared) -> Result<(), MemoryError> {
    write_sidecar(
        directory,
        &prepared.name,
        &GallerySidecar {
            schema: SCHEMA,
            owner: prepared.owner.clone(),
            picture: format!("{}.png", prepared.name),
            members: prepared.members.clone(),
            pictures: prepared.pictures.clone(),
            columns: prepared.columns,
            rows: prepared.rows,
        },
    )
}

/// The memory lines the host's log still holds (`WORLD memory_put`, `memory_deleted` and
/// `memory_composed`), oldest first: those of `query.owner` when given, the newest `query.limit`.
pub(super) fn history(log: &EventLog, query: &HistoryQuery) -> Result<History, MemoryError> {
    if let Some(owner) = &query.owner {
        check_owner(owner)?;
    }
    let mut entries: Vec<HistoryEntry> = log
        .after_sequence_in(0, usize::MAX, "WORLD")
        .into_iter()
        .filter(|line| {
            let mut words = line.text.split(' ').skip(1);
            matches!(
                words.next(),
                Some("memory_put" | "memory_deleted" | "memory_composed")
            ) && query.owner.as_ref().is_none_or(|owner| {
                line.text
                    .split(' ')
                    .any(|word| word.strip_prefix("owner=") == Some(owner.as_str()))
            })
        })
        .map(|line| HistoryEntry {
            sequence: line.sequence,
            at_ms: line.at_ms,
            line: line.text,
        })
        .collect();
    if let Some(limit) = query.limit {
        entries.drain(..entries.len().saturating_sub(limit));
    }
    Ok(History { entries })
}
