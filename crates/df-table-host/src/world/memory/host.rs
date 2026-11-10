//! What a host feature that remembers calls: it keeps a memory for an owner and reads an owner's
//! newest memories of a kind, through the same operations and the same audit line the routes use.
//! A feature never reaches the store any other way.

use super::operations::put;
use super::request::MemoryError;
use super::routes::{paths, record_put};
use super::store::{memory_directory, read_book};
use super::{Memory, MemoryDraft, MemoryFilter, Put, select};
use crate::blocking::run_blocking;
use crate::state::AppState;

/// Puts `draft` into its owner's memories, as `POST /dev/world/memory/put` does, and records it.
pub(crate) async fn keep(app: &AppState, draft: MemoryDraft) -> Result<Put, MemoryError> {
    let paths = paths(app);
    let kept = run_blocking(move || put(&paths, draft), MemoryError::Stopped).await?;
    record_put(app, &kept);
    Ok(kept)
}

/// The newest `count` memories of `kind` that `owner` keeps, oldest first; none when the owner
/// keeps none.
pub(crate) async fn recent(
    app: &AppState,
    owner: &str,
    kind: &str,
    count: usize,
) -> Result<Vec<Memory>, MemoryError> {
    let (directory, owner, kind) = (memory_directory(app), owner.to_owned(), kind.to_owned());
    run_blocking(
        move || {
            let book = match read_book(&directory, &owner) {
                Ok(book) => book,
                Err(MemoryError::UnknownOwner { .. }) => return Ok(Vec::new()),
                Err(error) => return Err(error),
            };
            let filter = MemoryFilter {
                kind: Some(kind),
                ..MemoryFilter::default()
            };
            let chosen = select(&book, &filter);
            let skipped = chosen.len().saturating_sub(count);
            Ok(chosen.into_iter().skip(skipped).cloned().collect())
        },
        MemoryError::Stopped,
    )
    .await
}
