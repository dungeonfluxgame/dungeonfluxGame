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
    photographic_only: bool,
}

impl ReferenceStore {
    pub(in crate::gameplay) fn new(generated_directory: PathBuf) -> Self {
        Self {
            directory: generated_directory,
            photographic_only: false,
        }
    }

    /// Creates a store for scene plate references. Base generated art was authored as painted
    /// concept art, so it must never be sent to a photographic plate request. Re-rendered live
    /// variants use the same stem with a `-documentary-photo` suffix, after visual review.
    pub(in crate::gameplay) fn photographic(generated_directory: PathBuf) -> Self {
        Self {
            directory: generated_directory,
            photographic_only: true,
        }
    }

    pub(in crate::gameplay) fn photographic_only(&self) -> bool {
        self.photographic_only
    }

    /// Returns the checked-in empty photographic carrier used only to satisfy the image edit
    /// endpoint when no live identity references have been generated yet. It has no scene,
    /// subject or composition for the provider to copy.
    pub(in crate::gameplay) fn neutral_photographic(
        &self,
    ) -> Result<ReferenceImage, ReferenceError> {
        if !self.photographic_only {
            return Err(ReferenceError::Missing);
        }
        let path = self
            .directory
            .parent()
            .ok_or(ReferenceError::Missing)?
            .join("scenes")
            .join("neutral-photographic-carrier.png");
        let bytes = read_file(&path)?;
        Ok(ReferenceImage::new(ImageFormat::Png, bytes))
    }

    /// Reads the generated art named by `path`: a served path such as
    /// `/assets/ui/generated/scene-place-inn-taproom.webp`, or a bare stem such as
    /// `sprite-patron-merchant` (read as `.webp`, then `.png`). Only the last path segment is
    /// used, and it must be lowercase letters, digits, `-` and `_` with an optional `.webp` or
    /// `.png`.
    pub(in crate::gameplay) fn load(&self, path: &str) -> Result<ReferenceImage, ReferenceError> {
        self.load_named(path, &[])
    }

    /// Reads only a generated photographic variant. The base stem is deliberately not a
    /// fallback: its existing bytes may be painterly and would contaminate the plate.
    pub(in crate::gameplay) fn load_photographic(
        &self,
        path: &str,
    ) -> Result<ReferenceImage, ReferenceError> {
        // A generated photographic variant may be emitted as PNG even when the source asset
        // was named with a `.webp` extension. Preserve the stem while allowing either provider
        // container for the variant.
        let stem = match path.rsplit_once('.') {
            Some((stem, "webp" | "png")) => stem,
            _ => path,
        };
        self.load_named(stem, &["-documentary-photo"])
    }

    fn load_named(&self, path: &str, suffixes: &[&str]) -> Result<ReferenceImage, ReferenceError> {
        let name = path.rsplit('/').next().unwrap_or_default();
        let (stem, extension) = match name.rsplit_once('.') {
            Some((stem, extension @ ("webp" | "png"))) => (stem, Some(extension)),
            Some(_) => return Err(ReferenceError::Name),
            None => (name, None),
        };
        if stem.is_empty()
            || stem.len() > 96
            || !stem.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
        {
            return Err(ReferenceError::Name);
        }
        let extensions: &[&str] = match extension {
            Some("png") => &["png"],
            Some(_) => &["webp"],
            None => &["webp", "png"],
        };
        let suffixes = suffixes
            .iter()
            .copied()
            .chain(suffixes.is_empty().then_some(""));
        for suffix in suffixes {
            for extension in extensions {
                let file = self.directory.join(format!("{stem}{suffix}.{extension}"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photographic_references_do_not_fall_back_to_the_painted_base_stem() {
        let directory = std::env::temp_dir().join(format!(
            "dungeonflux-photo-reference-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("temporary reference directory");
        std::fs::write(directory.join("scene-place-test.webp"), b"painted base")
            .expect("painted base fixture");
        let store = ReferenceStore::photographic(directory.clone());
        assert!(matches!(
            store.load_photographic("scene-place-test"),
            Err(ReferenceError::Missing)
        ));
        std::fs::write(
            directory.join("scene-place-test-photo.webp"),
            b"unreviewed CGI",
        )
        .expect("older nominal photo fixture");
        assert!(matches!(
            store.load_photographic("scene-place-test"),
            Err(ReferenceError::Missing)
        ));
        std::fs::write(
            directory.join("scene-place-test-documentary-photo.webp"),
            b"live photo",
        )
        .expect("photographic variant fixture");
        let photo = store
            .load_photographic("scene-place-test")
            .expect("photographic variant");
        assert_eq!(&photo.bytes[..], b"live photo");
        std::fs::remove_file(directory.join("scene-place-test-documentary-photo.webp"))
            .expect("remove webp fixture");
        std::fs::write(
            directory.join("scene-place-test-documentary-photo.png"),
            b"live png",
        )
        .expect("photographic png fixture");
        let photo = store
            .load_photographic("scene-place-test.webp")
            .expect("photographic png variant");
        assert_eq!(&photo.bytes[..], b"live png");
        std::fs::remove_dir_all(directory).expect("remove temporary reference directory");
    }

    #[test]
    fn neutral_photographic_carrier_is_provider_sized_and_nonempty() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated");
        let carrier = ReferenceStore::photographic(directory)
            .neutral_photographic()
            .expect("checked-in neutral photographic carrier");
        assert_eq!(carrier.format, ImageFormat::Png);
        assert!(!carrier.bytes.is_empty());
        assert!(carrier.bytes.len() as u64 <= MAXIMUM_REFERENCE_BYTES);
    }
}
