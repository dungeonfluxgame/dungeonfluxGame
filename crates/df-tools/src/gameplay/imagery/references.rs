//! Reference images for an edit, read from the served generated-art set.

use std::{
    io,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use df_providers::ImageFormat;
use sha2::{Digest, Sha256};

/// A reference is never larger than this; the provider adapter refuses bigger ones too.
pub(in crate::gameplay) const MAXIMUM_REFERENCE_BYTES: u64 = 4 * 1024 * 1024;

/// One reference image: its container, its bytes and the SHA-256 that identifies it in a key.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct ReferenceImage {
    pub(in crate::gameplay) format: ImageFormat,
    pub(in crate::gameplay) bytes: Bytes,
    pub(in crate::gameplay) sha256: String,
}

impl ReferenceImage {
    pub(in crate::gameplay) fn new(format: ImageFormat, bytes: Bytes) -> Self {
        let sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            format,
            bytes,
            sha256,
        }
    }
}

/// Why a reference could not be read. The path is never part of the error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum ReferenceError {
    /// The name is not a plain generated-art file name.
    Name,
    Missing,
    /// A symbolic link, a directory, an empty file or one over the size limit.
    Unusable,
}

/// The generated-art directory the server already serves under `/assets/ui/generated/`.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct ReferenceStore {
    directory: PathBuf,
}

impl ReferenceStore {
    pub(in crate::gameplay) fn new(generated_directory: PathBuf) -> Self {
        Self {
            directory: generated_directory,
        }
    }

    /// Reads the generated art named by `path`: a served path such as
    /// `/assets/ui/generated/scene-place-inn-taproom.webp`, or a bare stem such as
    /// `sprite-patron-merchant` (read as `.webp`, then `.png`). Only the last path segment is
    /// used, and it must be lowercase letters, digits, `-` and `_` with an optional `.webp` or
    /// `.png`.
    pub(in crate::gameplay) fn load(&self, path: &str) -> Result<ReferenceImage, ReferenceError> {
        let name = path.rsplit('/').next().unwrap_or_default();
        let (stem, extension) = match name.rsplit_once('.') {
            Some((stem, extension @ ("webp" | "png"))) => (stem, Some(extension)),
            Some(_) => return Err(ReferenceError::Name),
            None => (name, None),
        };
        if stem.is_empty()
            || stem.len() > 96
            || !stem
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_'))
        {
            return Err(ReferenceError::Name);
        }
        let candidates: &[&str] = match extension {
            Some("png") => &["png"],
            Some(_) => &["webp"],
            None => &["webp", "png"],
        };
        for extension in candidates {
            let file = self.directory.join(format!("{stem}.{extension}"));
            match read_file(&file) {
                Ok(bytes) => {
                    let format = if *extension == "png" {
                        ImageFormat::Png
                    } else {
                        ImageFormat::Webp
                    };
                    return Ok(ReferenceImage::new(format, bytes));
                }
                Err(ReferenceError::Missing) => {}
                Err(error) => return Err(error),
            }
        }
        Err(ReferenceError::Missing)
    }
}

fn read_file(path: &Path) -> Result<Bytes, ReferenceError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ReferenceError::Missing);
        }
        Err(_) => return Err(ReferenceError::Unusable),
    };
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAXIMUM_REFERENCE_BYTES {
        return Err(ReferenceError::Unusable);
    }
    std::fs::read(path)
        .map(Bytes::from)
        .map_err(|_| ReferenceError::Unusable)
}
