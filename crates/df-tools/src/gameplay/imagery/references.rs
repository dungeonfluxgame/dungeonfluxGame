//! Reference images for an edit, read from the served generated-art set.

use std::{
    io,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use df_providers::ImageFormat;
use df_render::art_contract;
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
    /// The name is a depiction the art contract does not allow as a reference: a painted stem, a
    /// role token or an identity key with no reviewed photograph. Nothing is read.
    NotPhotographic,
    Missing,
    /// A symbolic link, a directory, an empty file or one over the size limit.
    Unusable,
}

/// The reviewed photographs of the generated-art directory (see `df_render::art_contract`).
/// This is the only way a reference reaches an image-generation request, so no painted file can:
/// `load` resolves its argument through the contract and never opens any other file.
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

    /// Reads the reviewed photograph that answers `path`: a reviewed stem
    /// (`sprite-courier-documentary-photo`), an identity key that has one (`portrait-patron-merchant`) or
    /// a base stem whose `-documentary-photo` form is reviewed. Only the last path segment is
    /// used, and it must be lowercase letters, digits, `-` and `_` with an optional `.webp` or
    /// `.png`. Anything else that names a depiction is `NotPhotographic`.
    pub(in crate::gameplay) fn load(&self, path: &str) -> Result<ReferenceImage, ReferenceError> {
        let name = path.rsplit('/').next().unwrap_or_default();
        let stem = match name.rsplit_once('.') {
            Some((stem, "webp" | "png")) => stem,
            Some(_) => return Err(ReferenceError::Name),
            None => name,
        };
        if stem.is_empty()
            || stem.len() > 96
            || !stem.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
        {
            return Err(ReferenceError::Name);
        }
        let photo = art_contract::plan(art_contract::Role::GenerationReference, stem)
            .reviewed
            .ok_or(ReferenceError::NotPhotographic)?;
        for (extension, format) in [("webp", ImageFormat::Webp), ("png", ImageFormat::Png)] {
            match read_file(&self.directory.join(format!("{photo}.{extension}"))) {
                Ok(bytes) => return Ok(ReferenceImage::new(format, bytes)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("dungeonflux-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temporary reference directory");
        directory
    }

    #[test]
    fn the_loader_cannot_return_painted_bytes_even_when_the_file_is_on_disk() {
        let directory = directory("photo-reference");
        for painted in [
            "scene-place-test.webp",
            "scene-salt-quay-arrival-photo.webp",
            "portrait-ant-wolf.webp",
            "sprite-patron-bell-ringer.png",
            "figure-hero-1-photo.png",
            "ground-salt-quay.webp",
            "prop-crates.webp",
        ] {
            std::fs::write(directory.join(painted), b"painted").expect("painted fixture");
        }
        let store = ReferenceStore::new(directory.clone());
        for name in [
            "scene-place-test",
            "scene-salt-quay-arrival-photo",
            "portrait-ant-wolf",
            "sprite-patron-bell-ringer",
            "figure-hero-1-photo",
            "ground-salt-quay.webp",
            "assets/ui/generated/prop-crates",
            "live-room-photo",
        ] {
            assert_eq!(
                store.load(name).err(),
                Some(ReferenceError::NotPhotographic),
                "{name}"
            );
        }
        std::fs::remove_dir_all(directory).expect("remove temporary reference directory");
    }

    #[test]
    fn the_loader_reads_reviewed_photographs_by_stem_identity_key_or_base_stem() {
        let directory = directory("reviewed-reference");
        std::fs::write(
            directory.join("figure-hero-1-documentary-photo.webp"),
            b"hero webp",
        )
        .expect("reviewed webp fixture");
        std::fs::write(
            directory.join("sprite-courier-documentary-photo.png"),
            b"courier png",
        )
        .expect("reviewed png fixture");
        let store = ReferenceStore::new(directory.clone());
        let read = |name: &str| store.load(name).map(|image| image.bytes.to_vec());
        assert_eq!(
            read("figure-hero-1-documentary-photo"),
            Err(ReferenceError::NotPhotographic)
        );
        assert_eq!(
            read("portrait-hero-1"),
            Err(ReferenceError::NotPhotographic)
        );
        assert_eq!(
            read("/assets/ui/generated/figure-hero-1.webp"),
            Err(ReferenceError::NotPhotographic)
        );
        assert_eq!(read("sprite-courier"), Ok(b"courier png".to_vec()));
        assert_eq!(
            read("sprite-courier-documentary-photo.webp"),
            Ok(b"courier png".to_vec())
        );
        assert_eq!(read("sprite-bandit"), Err(ReferenceError::Missing));
        assert_eq!(read("../secret"), Err(ReferenceError::NotPhotographic));
        assert_eq!(read("Figure-Hero-1"), Err(ReferenceError::Name));
        std::fs::remove_dir_all(directory).expect("remove temporary reference directory");
    }
}
