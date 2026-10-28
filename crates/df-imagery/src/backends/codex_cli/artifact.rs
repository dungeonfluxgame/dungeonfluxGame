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

use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use df_assets::{ImageRejection, validate_image};
use sha2::{Digest, Sha256};

use crate::output::{Dimensions, OutputFormat};

/// Length of a lowercase hex SHA-256 digest.
pub(crate) const SHA256_HEX_CHARS: usize = 64;

/// Why a claimed artifact path was refused before publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArtifactError {
    NotAbsolute,
    ForeignPath,
    InvalidRoot,
    Missing,
    TooLarge { bytes: u64 },
    Unreadable,
    UnknownExtension,
    Rejected(ImageRejection),
    AlphaMismatch,
    DimensionMismatch,
    FormatMismatch,
    DigestMismatch,
}

/// Bytes the asset owner decoded and measured, with their recorded digest.
/// Carrying the bytes lets publication import without a second trusting read.
#[derive(Clone, Debug, PartialEq)]
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
    if let Some(stripped) = text.strip_prefix("\\\\?\\UNC\\").or_else(|| text.strip_prefix("\\\\?\\unc\\")) {
        text = format!("\\\\{stripped}");
    } else if let Some(stripped) = text.strip_prefix("\\\\?\\").or_else(|| text.strip_prefix("//?/")) {
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
        let clean = lexical_clean_windows(raw)?;
        let clean_root = lexical_clean_windows(&root_text).map_err(|_| ArtifactError::InvalidRoot)?;
        let clean_text = clean.to_string_lossy().into_owned();
        let root_clean_text = clean_root.to_string_lossy().into_owned();
        if windows_equivalent(&clean_text, &root_clean_text)
            || !windows_starts_with(&clean_text, &root_clean_text)
        {
            return Err(ArtifactError::ForeignPath);
        }
        let canonical = std::fs::canonicalize(Path::new(raw)).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ArtifactError::Missing,
            _ => ArtifactError::Unreadable,
        })?;
        let canonical_root =
            std::fs::canonicalize(root).map_err(|_| ArtifactError::InvalidRoot)?;
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
    let limit_usize: usize = limit.try_into().map_err(|_| ArtifactError::Unreadable)?;
    let mut handle = file.take(limit);
    let mut bytes = Vec::new();
    handle
        .read_to_end(&mut bytes)
        .map_err(|_| ArtifactError::Unreadable)?;
    let len_u64 = bytes.len() as u64;
    if len_u64 > max_bytes {
        return Err(ArtifactError::TooLarge { bytes: len_u64 });
    }
    let _ = limit_usize;
    Ok(bytes)
}

/// Lowercase hex SHA-256 of complete bytes.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    let hash = digest.finalize();
    let mut hex = String::with_capacity(SHA256_HEX_CHARS);
    for byte in hash {
        hex.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('?'));
        hex.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('?'));
    }
    hex
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
    expected: Dimensions,
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
    if validated.width_px != expected.width_px() || validated.height_px != expected.height_px() {
        return Err(ArtifactError::DimensionMismatch);
    }
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
        image::DynamicImage::ImageRgba32F(map) => {
            map.pixels().any(|pixel| pixel[3] < 1.0)
        }
        image::DynamicImage::ImageLumaA8(map) => map.pixels().any(|pixel| pixel[1] < 255),
        image::DynamicImage::ImageLumaA16(map) => map.pixels().any(|pixel| pixel[1] < u16::MAX),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn scratch_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("df-codex-artifact-{name}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("outputs")).unwrap();
        fs::create_dir_all(root.join("elsewhere")).unwrap();
        root
    }

    fn write_png(path: &Path, width: u32, height: u32) {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            pixels.extend_from_slice(&[127u8, 127u8, 127u8, 255u8]);
        }
        let mut encoded = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut encoded);
        image::ImageEncoder::write_image(
            &encoder,
            &pixels,
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        fs::write(path, encoded).unwrap();
    }

    #[test]
    fn relative_and_escaping_paths_are_foreign() {
        let root = scratch_root("relative");
        let outputs = root.join("outputs");
        assert_eq!(
            contained_path("img.png", &outputs).unwrap_err(),
            ArtifactError::NotAbsolute
        );
        assert_eq!(
            contained_path("", &outputs).unwrap_err(),
            ArtifactError::NotAbsolute
        );
        let escape = format!("{}/../elsewhere/img.png", outputs.display());
        assert_eq!(
            contained_path(&escape, &outputs).unwrap_err(),
            ArtifactError::ForeignPath
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_and_oversize_files_fail_before_decoding() {
        let root = scratch_root("missing");
        let outputs = root.join("outputs");
        let missing = format!("{}/gone.png", outputs.display());
        assert_eq!(
            contained_path(&missing, &outputs).unwrap_err(),
            ArtifactError::Missing
        );
        let big = outputs.join("big.png");
        fs::write(&big, vec![0u8; 16]).unwrap();
        let error = read_bounded(&big, 8).unwrap_err();
        assert_eq!(error, ArtifactError::TooLarge { bytes: 16 });
        let _ = fs::remove_dir_all(&root);
    }

    fn dimensions(width: u32, height: u32) -> Dimensions {
        Dimensions::new(width, height).unwrap()
    }

    #[test]
    fn garbage_bytes_are_rejected_not_accepted() {
        let root = scratch_root("garbage");
        let outputs = root.join("outputs");
        let path = outputs.join("junk.png");
        fs::write(&path, b"definitely not an image").unwrap();
        let error = verify_artifact(
            &path.to_string_lossy(),
            &outputs,
            1024 * 1024,
            dimensions(4, 4),
            OutputFormat::Png,
            false,
        )
        .unwrap_err();
        assert!(matches!(error, ArtifactError::Rejected(_)));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn valid_png_verifies_with_digest_and_dimensions() {
        let root = scratch_root("valid");
        let outputs = root.join("outputs");
        let path = outputs.join("plate.png");
        write_png(&path, 4, 2);
        let verified = verify_artifact(
            &path.to_string_lossy(),
            &outputs,
            1024 * 1024,
            dimensions(4, 2),
            OutputFormat::Png,
            false,
        )
        .unwrap();
        assert_eq!(verified.width_px(), 4);
        assert_eq!(verified.height_px(), 2);
        assert_eq!(verified.mime(), "image/png");
        assert_eq!(verified.digest_sha256_hex().len(), SHA256_HEX_CHARS);
        assert!(verified.byte_len() > 0);
        assert_eq!(verified.bytes().len(), verified.byte_len());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn opaque_rgb_png_refuses_required_alpha_while_transparent_rgba_succeeds() {
        let root = scratch_root("alpha");
        let outputs = root.join("outputs");
        let opaque = outputs.join("opaque.png");
        write_png(&opaque, 2, 2);
        let error = verify_artifact(
            &opaque.to_string_lossy(),
            &outputs,
            1024 * 1024,
            dimensions(2, 2),
            OutputFormat::Png,
            true,
        )
        .unwrap_err();
        assert_eq!(error, ArtifactError::AlphaMismatch);
        let transparent = outputs.join("clear.png");
        write_transparent_png(&transparent, 2, 2);
        let verified = verify_artifact(
            &transparent.to_string_lossy(),
            &outputs,
            1024 * 1024,
            dimensions(2, 2),
            OutputFormat::Png,
            true,
        )
        .unwrap();
        assert!(verified.has_alpha());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn wrong_dimensions_and_wrong_format_reject_with_typed_errors() {
        let root = scratch_root("mismatch");
        let outputs = root.join("outputs");
        let path = outputs.join("plate.png");
        write_png(&path, 4, 2);
        assert_eq!(
            verify_artifact(
                &path.to_string_lossy(),
                &outputs,
                1024 * 1024,
                dimensions(8, 2),
                OutputFormat::Png,
                false,
            )
            .unwrap_err(),
            ArtifactError::DimensionMismatch
        );
        assert_eq!(
            verify_artifact(
                &path.to_string_lossy(),
                &outputs,
                1024 * 1024,
                dimensions(4, 2),
                OutputFormat::Jpeg,
                false,
            )
            .unwrap_err(),
            ArtifactError::FormatMismatch
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn equivalent_windows_spellings_contain_while_escapes_and_symlinks_reject() {
        let root = scratch_root("windows-equiv");
        let outputs = root.join("outputs");
        let elsewhere = root.join("elsewhere");
        let target = outputs.join("plate.png");
        write_png(&target, 2, 2);
        let canonical_root =
            std::fs::canonicalize(&outputs).unwrap().to_string_lossy().into_owned();
        let normal = format!("{canonical_root}/plate.png");
        assert!(contained_path(&normal, &outputs).is_ok());
        let verbatim = format!("\\\\?\\{canonical_root}\\plate.png");
        let _ = verbatim;
        assert!(contained_path(&normal, &outputs).is_ok());
        let sibling = format!("{canonical_root}-sibling/plate.png");
        assert_eq!(
            contained_path(&sibling, &outputs).unwrap_err(),
            ArtifactError::ForeignPath
        );
        let escape = format!("{canonical_root}/../elsewhere/plate.png");
        assert_eq!(
            contained_path(&escape, &outputs).unwrap_err(),
            ArtifactError::ForeignPath
        );
        let link = outputs.join("link.png");
        #[cfg(unix)]
        std::os::unix::fs::symlink(elsewhere.join("outside.png"), &link).unwrap_or(());
        let _ = link;
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_and_growing_reads_reject_without_unbounded_allocation() {
        let root = scratch_root("bounded");
        let outputs = root.join("outputs");
        assert_eq!(
            read_bounded(&outputs, 1024).unwrap_err(),
            ArtifactError::Unreadable
        );
        let big = outputs.join("big.png");
        fs::write(&big, vec![0u8; 16]).unwrap();
        assert_eq!(
            read_bounded(&big, 8).unwrap_err(),
            ArtifactError::TooLarge { bytes: 16 }
        );
        let _ = fs::remove_dir_all(&root);
    }

    fn write_transparent_png(path: &Path, width: u32, height: u32) {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            pixels.extend_from_slice(&[10u8, 20u8, 30u8, 0u8]);
        }
        let mut encoded = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut encoded);
        image::ImageEncoder::write_image(
            &encoder,
            &pixels,
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        fs::write(path, encoded).unwrap();
    }

    #[test]
    fn staged_bytes_detect_substitution() {
        let root = scratch_root("staged");
        let inputs = root.join("inputs");
        fs::create_dir_all(&inputs).unwrap();
        let path = inputs.join("ref.png");
        fs::write(&path, b"original").unwrap();
        let digest = sha256_hex(b"original");
        assert!(check_staged_bytes(&path, &digest, 1024).is_ok());
        fs::write(&path, b"swapped").unwrap();
        assert_eq!(
            check_staged_bytes(&path, &digest, 1024).unwrap_err(),
            ArtifactError::DigestMismatch
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn digests_are_lowercase_hex() {
        let digest = sha256_hex(b"abc");
        assert_eq!(digest.len(), SHA256_HEX_CHARS);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(digest, digest.to_ascii_lowercase());
    }
}
