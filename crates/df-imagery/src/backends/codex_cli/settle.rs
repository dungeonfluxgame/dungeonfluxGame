//! Settlement: binds a correlated completed turn to verified imported bytes.
//!
//! A `Ready` receipt names the imported artifact in the owned outputs
//! directory, never the original mutable helper path. Import copies the exact
//! verified bytes under a sanitized item-derived name with create-new
//! semantics, then revalidates the written object with bounded reads; every
//! step runs under the turn deadline, and the blocking decode stays owned so
//! a late result is never treated as a completed verification.

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::artifact::{ArtifactError, VerifiedArtifact, sha256_hex, verify_artifact};
use super::protocol::{ImageItem, TurnCompletedView, match_inputs};
use super::{
    AlphaRequirement, ArtifactReceipt, CodexError, CodexImageRequest, CodexOutcome,
    GenerationFailureKind, UnknownReason, UnsupportedKind,
};

/// How many suffixed import names the adapter tries before refusing: the
/// outputs directory is service-owned, so exhaustion means hostile squatting,
/// not a missing feature.
const MAXIMUM_IMPORT_ATTEMPTS: u32 = 64;

pub(crate) async fn settle_turn(
    view: TurnCompletedView,
    request: &CodexImageRequest,
    image_paths: &[String],
    outputs_root: &Path,
    max_artifact_bytes: u64,
    deadline_at: Instant,
) -> CodexOutcome {
    if view.status != "completed" {
        if view.status == "failed" {
            return CodexOutcome::Failed {
                error: CodexError::TurnFailed {
                    status: view.status,
                },
                usage: view.usage,
            };
        }
        if view.status == "interrupted" {
            return CodexOutcome::Unknown {
                reason: UnknownReason::Interrupted,
                usage: view.usage,
            };
        }
        return CodexOutcome::Unknown {
            reason: UnknownReason::LostOutput,
            usage: view.usage,
        };
    }
    let staged: Vec<(&str, &str)> = request
        .references
        .iter()
        .zip(image_paths.iter())
        .map(|(reference, sent)| (sent.as_str(), reference.sha256_hex()))
        .collect();
    let source_roots = source_roots(outputs_root, &view.thread_id);
    let mut first_artifact_error: Option<ArtifactError> = None;
    let mut first_failure: Option<CodexError> = None;
    let mut saw_file_id = false;
    let mut saw_result = false;
    let mut saw_nonterminal = false;
    for item in &view.items {
        if let Err(error) = match_inputs(&item.input_images, &staged) {
            return CodexOutcome::Failed {
                error: CodexError::Protocol(error),
                usage: view.usage,
            };
        }
        let item_failure = item
            .failure
            .clone()
            .map(|failure| CodexError::GenerationFailed {
                kind: match failure {
                    super::protocol::ItemFailure::UsageLimited { limit_id } => {
                        GenerationFailureKind::UsageLimited { limit_id }
                    }
                    super::protocol::ItemFailure::Reported { kind } => {
                        GenerationFailureKind::Reported { kind }
                    }
                },
            });
        if item.status != "completed" {
            saw_nonterminal = true;
            if first_failure.is_none() {
                first_failure = item_failure;
            }
            continue;
        }
        if item_failure.is_some() {
            if first_failure.is_none() {
                first_failure = item_failure;
            }
            continue;
        }
        if let Some(path) = &item.saved_path {
            let candidate = verify_from_sources(
                path,
                &source_roots,
                outputs_root,
                max_artifact_bytes,
                request,
                item,
                deadline_at,
            )
            .await;
            match candidate {
                Ok(verified) => {
                    return CodexOutcome::Ready {
                        receipt: ArtifactReceipt {
                            thread_id: view.thread_id,
                            turn_id: view.turn_id,
                            item_id: item.id.clone(),
                            file_id: item.file_id.clone(),
                            path: verified.path().to_owned(),
                            digest_sha256_hex: verified.digest_sha256_hex().to_owned(),
                            mime: verified.mime().to_owned(),
                            width_px: verified.width_px(),
                            height_px: verified.height_px(),
                            byte_len: verified.byte_len(),
                            // Ground truth from decoded pixels, not the
                            // optional item metadata, which may be missing
                            // or contradict the bytes already checked.
                            transparent_background: Some(verified.has_alpha()),
                            revised_prompt_present: item.revised_prompt_present,
                            candidate_count: view.items.len(),
                        },
                        usage: view.usage,
                    };
                }
                Err(error) => {
                    if first_artifact_error.is_none() {
                        first_artifact_error = Some(error);
                    }
                }
            }
        } else if item.file_id.is_some() {
            saw_file_id = true;
        } else if item.has_result {
            saw_result = true;
        }
    }
    if let Some(error) = first_artifact_error {
        return CodexOutcome::Failed {
            error: CodexError::Artifact(error),
            usage: view.usage,
        };
    }
    if let Some(error) = first_failure {
        return CodexOutcome::Failed {
            error,
            usage: view.usage,
        };
    }
    if saw_file_id {
        return CodexOutcome::Failed {
            error: CodexError::Unsupported {
                kind: UnsupportedKind::FileIdOnly,
            },
            usage: view.usage,
        };
    }
    if saw_result {
        return CodexOutcome::Failed {
            error: CodexError::Unsupported {
                kind: UnsupportedKind::ResultEncoding,
            },
            usage: view.usage,
        };
    }
    if saw_nonterminal {
        return CodexOutcome::Unknown {
            reason: UnknownReason::LostOutput,
            usage: view.usage,
        };
    }
    CodexOutcome::Failed {
        error: CodexError::NoArtifact,
        usage: view.usage,
    }
}

/// Where a completed image may legitimately live: the service's outputs directory, where the
/// model was asked to write, and Codex's own generated-image folder for this thread, where the
/// image tool saves in the read-only sandbox. Either way the bytes are verified in place and
/// then imported into the outputs directory, so the receipt never names a source path.
fn source_roots(outputs_root: &Path, thread_id: &str) -> Vec<PathBuf> {
    let mut roots = vec![outputs_root.to_owned()];
    if let Some(generated) = generated_images_root(thread_id) {
        roots.push(generated);
    }
    roots
}

/// `<codex home>/generated_images/<thread id>`, or `None` when the thread id is not a plain
/// identifier or no home directory is known. `CODEX_HOME` wins, as it does for the CLI.
pub(crate) fn generated_images_root(thread_id: &str) -> Option<PathBuf> {
    let plain = !thread_id.is_empty()
        && thread_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !plain {
        return None;
    }
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".codex")))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))?;
    Some(home.join("generated_images").join(thread_id))
}

/// Tries each source root in order. A path outside one root is not a failure yet; the next
/// root decides. Any other verdict, including a missing or corrupt file, is final.
async fn verify_from_sources(
    raw_path: &str,
    source_roots: &[PathBuf],
    outputs_root: &Path,
    max_bytes: u64,
    request: &CodexImageRequest,
    item: &ImageItem,
    deadline_at: Instant,
) -> Result<VerifiedArtifact, ArtifactError> {
    let mut verdict = Err(ArtifactError::ForeignPath);
    for root in source_roots {
        verdict = verify_candidate(
            raw_path,
            root,
            outputs_root,
            max_bytes,
            request,
            item,
            deadline_at,
        )
        .await;
        if !matches!(verdict, Err(ArtifactError::ForeignPath)) {
            break;
        }
    }
    verdict
}

async fn verify_candidate(
    raw_path: &str,
    source_root: &Path,
    outputs_root: &Path,
    max_bytes: u64,
    request: &CodexImageRequest,
    item: &ImageItem,
    deadline_at: Instant,
) -> Result<VerifiedArtifact, ArtifactError> {
    let owned_path = raw_path.to_owned();
    let owned_root = source_root.to_owned();
    let format = request.format;
    let require_alpha = request.alpha == AlphaRequirement::PreserveAlpha;
    // The deadline prevents new work: an expired turn never spawns the
    // blocking decode. Otherwise the handle stays owned below.
    if Instant::now() >= deadline_at {
        return Err(ArtifactError::Unreadable);
    }
    let handle = tokio::task::spawn_blocking(move || {
        verify_artifact(&owned_path, &owned_root, max_bytes, format, require_alpha)
    });
    let mut slot = Some(handle);
    let verified = match join_blocking(&mut slot, deadline_at).await {
        Some(Ok(verified)) => verified,
        Some(Err(error)) => return Err(error),
        None => return Err(ArtifactError::Unreadable),
    };
    if verified.mime() != format.mime() {
        return Err(ArtifactError::FormatMismatch);
    }
    if require_alpha && !verified.has_alpha() {
        return Err(ArtifactError::AlphaMismatch);
    }
    import_verified_bytes(&verified, outputs_root, item, max_bytes, deadline_at).await
}

/// Awaits owned blocking filesystem work under the turn deadline without ever
/// detaching it: the handle stays in the caller-owned slot while the deadline
/// runs, and on expiry it is aborted then rejoined before the caller maps the
/// timeout. A completed handle is consumed without a second poll; a late
/// result is never reported as completed. Callers map `None` to their own
/// timeout outcome.
async fn join_blocking<T>(
    slot: &mut Option<tokio::task::JoinHandle<T>>,
    deadline_at: Instant,
) -> Option<T> {
    let handle = slot.as_mut()?;
    match tokio::time::timeout_at(deadline_at.into(), &mut *handle).await {
        Ok(value) => {
            let _ = slot.take();
            // A failed owned blocking task has no artifact result to accept.
            value.ok()
        }
        Err(_) => {
            handle.abort();
            let _ = (&mut *handle).await;
            let _ = slot.take();
            None
        }
    }
}

fn import_stem(item_id: &str) -> String {
    let mut stem = String::with_capacity(item_id.len());
    for character in item_id.chars() {
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            stem.push(character);
        }
    }
    if stem.is_empty() {
        stem.push_str("artifact");
    }
    stem
}

/// Bounded async read matching the owned-output bound: at most max_bytes + 1
/// are observed, and anything beyond max_bytes refuses with the observed
/// prefix length instead of allocating the whole file.
async fn read_bounded_async(path: &Path, max_bytes: u64) -> Result<Vec<u8>, ArtifactError> {
    use tokio::io::AsyncReadExt;

    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ArtifactError::Missing,
            _ => ArtifactError::Unreadable,
        })?;
    let metadata = file
        .metadata()
        .await
        .map_err(|_| ArtifactError::Unreadable)?;
    if !metadata.is_file() {
        return Err(ArtifactError::Unreadable);
    }
    let limit = max_bytes.checked_add(1).ok_or(ArtifactError::Unreadable)?;
    let mut take = file.take(limit);
    let mut bytes = Vec::new();
    take.read_to_end(&mut bytes)
        .await
        .map_err(|_| ArtifactError::Unreadable)?;
    if bytes.len() as u64 > max_bytes {
        return Err(ArtifactError::TooLarge {
            bytes: bytes.len() as u64,
        });
    }
    Ok(bytes)
}

/// Imports the exact verified bytes into the owned outputs directory and
/// returns the imported artifact: the receipt path and digest identify the
/// import, never the original mutable helper path. The import uses
/// create-new writes so an existing link or directory is never followed, and
/// revalidates the written object with bounded reads. The deadline only
/// prevents starting new work: every in-progress filesystem operation runs to
/// completion, and only a destination this attempt created is ever removed.
/// Cleanup may therefore finish after the deadline, and the expired outcome
/// is returned only after that owned cleanup.
async fn import_verified_bytes(
    verified: &VerifiedArtifact,
    outputs_root: &Path,
    item: &ImageItem,
    max_bytes: u64,
    deadline_at: Instant,
) -> Result<VerifiedArtifact, ArtifactError> {
    let extension = match verified.mime() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => return Err(ArtifactError::Unreadable),
    };
    let stem = import_stem(&item.id);
    let expected_digest = verified.digest_sha256_hex().to_owned();
    let expected_len = verified.byte_len();
    let mut attempt: u32 = 0;
    loop {
        if attempt >= MAXIMUM_IMPORT_ATTEMPTS {
            return Err(ArtifactError::Unreadable);
        }
        if Instant::now() >= deadline_at {
            return Err(ArtifactError::Unreadable);
        }
        let name = if attempt == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}-{attempt}.{extension}")
        };
        attempt += 1;
        let destination = outputs_root.join(&name);
        // The stem is sanitized, so this always holds; the check keeps
        // confinement explicit at the construction site.
        if !destination.starts_with(outputs_root) {
            return Err(ArtifactError::Unreadable);
        }
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .await
        {
            Ok(file) => {
                // This destination was just created by this attempt, so a
                // failure removes exactly this file and never a pre-existing
                // collision.
                if write_and_confirm(file, &destination, verified, max_bytes, deadline_at).await {
                    return Ok(verified.imported_at(destination));
                }
                remove_failed_import(&destination).await;
                return Err(ArtifactError::Unreadable);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if Instant::now() >= deadline_at {
                    return Err(ArtifactError::Unreadable);
                }
                // Never follow an existing link or directory: only a
                // regular file with the exact verified digest is reused, and
                // a pre-existing collision is never removed.
                let metadata = tokio::fs::symlink_metadata(&destination)
                    .await
                    .map_err(|_| ArtifactError::Unreadable)?;
                if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                    continue;
                }
                match read_bounded_async(&destination, max_bytes).await {
                    Ok(current)
                        if sha256_hex(&current) == expected_digest
                            && current.len() == expected_len
                            && Instant::now() < deadline_at =>
                    {
                        return Ok(verified.imported_at(destination));
                    }
                    Ok(_) => continue,
                    Err(_) => {
                        if Instant::now() >= deadline_at {
                            return Err(ArtifactError::Unreadable);
                        }
                        continue;
                    }
                }
            }
            Err(_) => return Err(ArtifactError::Unreadable),
        }
    }
}

/// Writes the verified bytes into a file this attempt just created, then
/// rereads them with a bounded read. True only when the reread bytes match
/// the verified digest and length before the deadline. The file handle is
/// closed before returning so a failed import can be removed.
async fn write_and_confirm(
    mut file: tokio::fs::File,
    destination: &Path,
    verified: &VerifiedArtifact,
    max_bytes: u64,
    deadline_at: Instant,
) -> bool {
    use tokio::io::AsyncWriteExt;

    if Instant::now() >= deadline_at
        || file.write_all(verified.bytes()).await.is_err()
        || file.flush().await.is_err()
    {
        return false;
    }
    drop(file);
    if Instant::now() >= deadline_at {
        return false;
    }
    matches!(
        read_bounded_async(destination, max_bytes).await,
        Ok(reread) if sha256_hex(&reread) == verified.digest_sha256_hex()
            && reread.len() == verified.byte_len()
            && Instant::now() < deadline_at
    )
}

/// Removes a destination this attempt created but could not confirm.
async fn remove_failed_import(destination: &Path) {
    // A leftover file is harmless: it is never named by a receipt, the import
    // has already failed, and the next attempt never reuses an unverified file.
    let _ = tokio::fs::remove_file(destination).await;
}

#[cfg(test)]
#[path = "settle_tests.rs"]
mod tests;
