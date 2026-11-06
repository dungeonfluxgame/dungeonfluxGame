//! Owned-output artifact verification for Codex image receipts.
//!
//! A `savedPath` from the CLI is a normalized absolute path claim, not proof:
//! it may point outside the attempt, at missing bytes, or at undecodable
//! content. This module confines the claim to the service-owned output
//! directory (lexically on normalized Windows-equivalent forms, then through
//! canonical symlink resolution), reads one open handle under a max-plus-one
//! bound with checked arithmetic, records its SHA-256, runs the asset owner
//! decoder for MIME, pixel bounds and dimensions, and scans decoded pixels
//! for actual transparency when the request requires alpha.
//!
//! What stays explicit: the digest is recorded, never compared against a
//! server value that does not exist; symlink resolution narrows but cannot
//! fully close a replace-between-check-and-read race, so the service imports
//! verified bytes into the owned directory and revalidates the digest.

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use df_assets::{ImageRejection, validate_image};
use sha2::{Digest, Sha256};

use crate::output::OutputFormat;

/// Length of a lowercase hex SHA-256 digest.
pub(crate) const SHA256_HEX_CHARS: usize = 64;

/// Why a claimed artifact path was refused before publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArtifactError {
    NotAbsolute,
    ForeignPath,
    InvalidRoot,
    Missing,
    /// Observed prefix length (`max_bytes + 1`): a truthful lower bound on
    /// the file length, never the full length, because reading stays bounded.
    TooLarge {
        bytes: u64,
    },
    Unreadable,
    UnknownExtension,
    Rejected(ImageRejection),
    AlphaMismatch,
    FormatMismatch,
    DigestMismatch,
}

/// Bytes the asset owner decoded and measured, with their recorded digest.
/// Carrying the bytes lets publication import without a second trusting read.
#[derive(Clone, PartialEq)]
pub(crate) struct VerifiedArtifact {
    path: PathBuf,
    digest_sha256_hex: String,
    mime: String,
    width_px: u32,
    height_px: u32,
    byte_len: usize,
    bytes: Vec<u8>,
    has_alpha: bool,
}

/// Debug keeps routing facts and the digest but never the image bytes: this
/// is the verified-data boundary, so a format here must not dump raster
/// content into logs or diagnostics.
impl fmt::Debug for VerifiedArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifiedArtifact")
            .field("path", &self.path)
            .field("digest_sha256_hex", &self.digest_sha256_hex)
            .field("mime", &self.mime)
            .field("width_px", &self.width_px)
            .field("height_px", &self.height_px)
            .field("byte_len", &self.byte_len)
            .field("has_alpha", &self.has_alpha)
            .finish()
    }
}

impl VerifiedArtifact {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn digest_sha256_hex(&self) -> &str {
        &self.digest_sha256_hex
    }

    pub(crate) fn mime(&self) -> &str {
        &self.mime
    }

    pub(crate) fn width_px(&self) -> u32 {
        self.width_px
    }

    pub(crate) fn height_px(&self) -> u32 {
        self.height_px
    }

    pub(crate) fn byte_len(&self) -> usize {
        self.byte_len
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn has_alpha(&self) -> bool {
        self.has_alpha
    }

    /// Rebinds verified bytes to their imported location: digest, decoded
    /// facts and bytes are unchanged, only the identifying path moves from
    /// the mutable helper claim to the owned service import.
    pub(crate) fn imported_at(&self, path: PathBuf) -> Self {
        Self {
            path,
            digest_sha256_hex: self.digest_sha256_hex.clone(),
            mime: self.mime.clone(),
            width_px: self.width_px,
            height_px: self.height_px,
            byte_len: self.byte_len,
            bytes: self.bytes.clone(),
            has_alpha: self.has_alpha,
        }
    }
}

fn lexical_clean(path: &Path) -> Result<PathBuf, ArtifactError> {
    if !path.is_absolute() {
        if is_windows_absolute(&path.to_string_lossy()) {
            return lexical_clean_windows(&path.to_string_lossy());
        }
        return Err(ArtifactError::NotAbsolute);
    }
    if is_windows_path(&path.to_string_lossy()) {
        return lexical_clean_windows(&path.to_string_lossy());
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => clean.push(prefix.as_os_str()),
            Component::RootDir => clean.push(Component::RootDir),
            Component::CurDir => {}
            Component::ParentDir => {
                if !clean.pop() {
                    return Err(ArtifactError::ForeignPath);
                }
            }
            Component::Normal(part) => clean.push(part),
        }
    }
    Ok(clean)
}

fn is_windows_absolute(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') {
        return bytes[0].is_ascii_alphabetic();
    }
    text.starts_with("\\\\?\\") || text.starts_with("//?/")
}

fn is_windows_path(text: &str) -> bool {
    text.contains('\\') || is_windows_absolute(text)
}

fn lexical_clean_windows(raw: &str) -> Result<PathBuf, ArtifactError> {
    let normalized = normalize_windows(raw);
    let mut parts: Vec<&str> = Vec::new();
    for part in normalized.split('\\') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if parts.pop().is_none() {
                return Err(ArtifactError::ForeignPath);
            }
            continue;
        }
        parts.push(part);
    }
    Ok(PathBuf::from(parts.join("\\")))
}

fn normalize_windows(raw: &str) -> String {
    let mut text = raw.replace('/', "\\");
    if let Some(stripped) = text
        .strip_prefix("\\\\?\\UNC\\")
        .or_else(|| text.strip_prefix("\\\\?\\unc\\"))
    {
        text = format!("\\\\{stripped}");
    } else if let Some(stripped) = text
        .strip_prefix("\\\\?\\")
        .or_else(|| text.strip_prefix("//?/"))
    {
        text = stripped.to_owned();
    }
    text.to_ascii_lowercase()
}

fn windows_equivalent(first: &str, second: &str) -> bool {
    normalize_windows(first) == normalize_windows(second)
}

fn windows_starts_with(path: &str, root: &str) -> bool {
    let path = normalize_windows(path);
    let root = normalize_windows(root);
    path == root || path.starts_with(&format!("{root}\\"))
}

/// Confines a claimed path to the owned output directory: lexical first on
/// normalized equivalent forms so `C:\...` and `\\?\C:\...` compare equal,
/// then canonical with the same equivalence so verbatim roots match.
pub(crate) fn contained_path(raw: &str, root: &Path) -> Result<PathBuf, ArtifactError> {
    if raw.is_empty() {
        return Err(ArtifactError::NotAbsolute);
    }
    let raw_windows = is_windows_path(raw);
    let root_text = root.to_string_lossy().into_owned();
    let root_windows = is_windows_path(&root_text);
    if raw_windows || root_windows {
        // A bare relative claim or a rooted-but-driveless claim (`\x`) is not
        // absolute on Windows; it is refused before confinement downgrades it.
        if !Path::new(raw).is_absolute() && !is_windows_absolute(raw) {
            return Err(ArtifactError::NotAbsolute);
        }
        if !root.is_absolute() && !is_windows_absolute(&root_text) {
            return Err(ArtifactError::InvalidRoot);
        }
        let clean = lexical_clean_windows(raw)?;
        let clean_root =
            lexical_clean_windows(&root_text).map_err(|_| ArtifactError::InvalidRoot)?;
        let clean_text = clean.to_string_lossy().into_owned();
        let root_clean_text = clean_root.to_string_lossy().into_owned();
        if windows_equivalent(&clean_text, &root_clean_text)
            || !windows_starts_with(&clean_text, &root_clean_text)
        {
            return Err(ArtifactError::ForeignPath);
        }
        let canonical =
            std::fs::canonicalize(Path::new(raw)).map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => ArtifactError::Missing,
                _ => ArtifactError::Unreadable,
            })?;
        let canonical_root = std::fs::canonicalize(root).map_err(|_| ArtifactError::InvalidRoot)?;
        let canonical_text = canonical.to_string_lossy().into_owned();
        let canonical_root_text = canonical_root.to_string_lossy().into_owned();
        if windows_equivalent(&canonical_text, &canonical_root_text)
            || !windows_starts_with(&canonical_text, &canonical_root_text)
        {
            return Err(ArtifactError::ForeignPath);
        }
        return Ok(canonical);
    }
    let clean = lexical_clean(Path::new(raw))?;
    let clean_root = lexical_clean(root).map_err(|_| ArtifactError::InvalidRoot)?;
    if clean == clean_root || !clean.starts_with(&clean_root) {
        return Err(ArtifactError::ForeignPath);
    }
    let canonical = std::fs::canonicalize(&clean).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ArtifactError::Missing,
        _ => ArtifactError::Unreadable,
    })?;
    let canonical_root =
        std::fs::canonicalize(&clean_root).map_err(|_| ArtifactError::InvalidRoot)?;
    if canonical == canonical_root || !canonical.starts_with(&canonical_root) {
        return Err(ArtifactError::ForeignPath);
    }
    Ok(canonical)
}

/// Reads one open regular-file handle under a max-plus-one bound with checked
/// arithmetic: a growing file cannot allocate beyond the bound before refusal.
/// `TooLarge` therefore reports the observed `max_bytes + 1` prefix length, a
/// truthful lower bound on the file length, not the full length.
pub(crate) fn read_bounded(path: &Path, max_bytes: u64) -> Result<Vec<u8>, ArtifactError> {
    let file = File::open(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ArtifactError::Missing,
        _ => ArtifactError::Unreadable,
    })?;
    let metadata = file.metadata().map_err(|_| ArtifactError::Unreadable)?;
    if !metadata.is_file() {
        return Err(ArtifactError::Unreadable);
    }
    let limit = max_bytes.checked_add(1).ok_or(ArtifactError::Unreadable)?;
    // The observed prefix must be addressable before any byte is read.
    usize::try_from(limit).map_err(|_| ArtifactError::Unreadable)?;
    let mut handle = file.take(limit);
    let mut bytes = Vec::new();
    handle
        .read_to_end(&mut bytes)
        .map_err(|_| ArtifactError::Unreadable)?;
    let len_u64 = bytes.len() as u64;
    if len_u64 > max_bytes {
        return Err(ArtifactError::TooLarge { bytes: len_u64 });
    }
    Ok(bytes)
}

/// Lowercase hex SHA-256 of complete bytes.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    crate::lower_hex(&Sha256::digest(bytes))
}

/// Re-reads a staged reference and compares it against the digest the service
/// recorded when staging, so substituted bytes fail before any CLI launch.
pub(crate) fn check_staged_bytes(
    path: &Path,
    expected_sha256_hex: &str,
    max_bytes: u64,
) -> Result<(), ArtifactError> {
    let bytes = read_bounded(path, max_bytes)?;
    if sha256_hex(&bytes) == expected_sha256_hex {
        Ok(())
    } else {
        Err(ArtifactError::DigestMismatch)
    }
}

fn extension_claim(file_name: &str) -> Result<(&'static str, &'static str), ArtifactError> {
    let extension = file_name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => Ok(("image/png", "png")),
        "jpg" | "jpeg" => Ok(("image/jpeg", "jpg")),
        "webp" => Ok(("image/webp", "webp")),
        _ => Err(ArtifactError::UnknownExtension),
    }
}

/// Verifies one claimed artifact: ownership, bounded complete bytes, asset
/// decoding, dimensions and format against the frozen request, and actual
/// pixel transparency when the request requires alpha.
pub(crate) fn verify_artifact(
    raw_path: &str,
    outputs_root: &Path,
    max_bytes: u64,
    format: OutputFormat,
    require_alpha: bool,
) -> Result<VerifiedArtifact, ArtifactError> {
    let path = contained_path(raw_path, outputs_root)?;
    let bytes = read_bounded(&path, max_bytes)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let (mime, extension) = extension_claim(file_name)?;
    let validated = validate_image(&bytes, mime, extension).map_err(ArtifactError::Rejected)?;
    if validated.mime != format.mime() {
        return Err(ArtifactError::FormatMismatch);
    }
    let has_alpha = decoded_has_transparency(&bytes);
    if require_alpha && !has_alpha {
        return Err(ArtifactError::AlphaMismatch);
    }
    let byte_len = bytes.len();
    Ok(VerifiedArtifact {
        path,
        digest_sha256_hex: sha256_hex(&bytes),
        mime: validated.mime.to_owned(),
        width_px: validated.width_px,
        height_px: validated.height_px,
        byte_len,
        bytes,
        has_alpha,
    })
}

fn decoded_has_transparency(bytes: &[u8]) -> bool {
    let Ok(image) = image::load_from_memory(bytes) else {
        return false;
    };
    match image {
        image::DynamicImage::ImageRgba8(map) => map.pixels().any(|pixel| pixel[3] < 255),
        image::DynamicImage::ImageRgba16(map) => map.pixels().any(|pixel| pixel[3] < u16::MAX),
        image::DynamicImage::ImageRgba32F(map) => map.pixels().any(|pixel| pixel[3] < 1.0),
        image::DynamicImage::ImageLumaA8(map) => map.pixels().any(|pixel| pixel[1] < 255),
        image::DynamicImage::ImageLumaA16(map) => map.pixels().any(|pixel| pixel[1] < u16::MAX),
        _ => false,
    }
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;
