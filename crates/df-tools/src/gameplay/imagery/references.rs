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
    origin: ReferenceOrigin,
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
            origin: ReferenceOrigin::Unbound,
        }
    }

    pub(in crate::gameplay) fn from_catalog(
        format: ImageFormat,
        bytes: Bytes,
        source: df_imagery::SourceRef,
    ) -> Result<Self, super::ImageFailure> {
        if <[u8; 32]>::from(Sha256::digest(&bytes)) != *source.digest.as_bytes() {
            return Err(super::ImageFailure::BadOutput);
        }
        let mut reference = Self::new(format, bytes);
        reference.origin = ReferenceOrigin::Catalog(source);
        Ok(reference)
    }

    pub(in crate::gameplay) fn from_trusted(
        format: ImageFormat,
        bytes: Bytes,
        origin: TrustedReferenceOrigin,
    ) -> Self {
        let mut reference = Self::new(format, bytes);
        reference.origin = ReferenceOrigin::Trusted(origin);
        reference
    }

    pub(in crate::gameplay) fn trusted_source(&self) -> Option<&TrustedReferenceOrigin> {
        match &self.origin {
            ReferenceOrigin::Trusted(origin) => Some(origin),
            _ => None,
        }
    }

    pub(in crate::gameplay) fn canonical_source(&self) -> Option<&df_imagery::SourceRef> {
        match &self.origin {
            ReferenceOrigin::Catalog(source) => Some(source),
            ReferenceOrigin::Unbound
            | ReferenceOrigin::Reviewed(_)
            | ReferenceOrigin::Trusted(_) => None,
        }
    }

    pub(in crate::gameplay) fn reviewed_source(&self) -> Option<&ReviewedReference> {
        match &self.origin {
            ReferenceOrigin::Reviewed(source) => Some(source),
            ReferenceOrigin::Unbound
            | ReferenceOrigin::Catalog(_)
            | ReferenceOrigin::Trusted(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
enum ReferenceOrigin {
    Unbound,
    Catalog(df_imagery::SourceRef),
    Reviewed(ReviewedReference),
    Trusted(TrustedReferenceOrigin),
}

/// Actual host ingress proof. It is never deserialized from an image request.
#[derive(Clone)]
pub(in crate::gameplay) enum TrustedReferenceOrigin {
    AuthoredPainting { root: PathBuf, painting: String },
    ApprovedPortrait(std::sync::Arc<dyn ApprovedPortraitSource>),
    AdventureManifest(AdventureManifest, super::ImageKey),
    CommittedPortrait(std::sync::Arc<dyn ApprovedPortraitSource>),
    DevelopmentSavedPhoto(std::sync::Arc<dyn DevelopmentSavedPhotoSource>),
}
impl std::fmt::Debug for TrustedReferenceOrigin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::AuthoredPainting { .. } => "TrustedReferenceOrigin::AuthoredPainting",
            Self::ApprovedPortrait(_) => "TrustedReferenceOrigin::ApprovedPortrait",
            Self::AdventureManifest(_, _) => "TrustedReferenceOrigin::AdventureManifest",
            Self::CommittedPortrait(_) => "TrustedReferenceOrigin::CommittedPortrait",
            Self::DevelopmentSavedPhoto(_) => "TrustedReferenceOrigin::DevelopmentSavedPhoto",
        })
    }
}
/// Implemented at the actual authenticated seat/library owner, retaining its current proof.
pub(in crate::gameplay) trait ApprovedPortraitSource: Send + Sync {
    fn read_current<'a>(
        &'a self,
        work_owner: &'a df_imagery::ImageOwner,
    ) -> super::central::ReferenceFuture<'a, ReferenceImage>;
}
impl TrustedReferenceOrigin {
    pub(in crate::gameplay) async fn read_current(
        &self,
        work_owner: &df_imagery::ImageOwner,
    ) -> Result<ReferenceImage, super::ImageFailure> {
        match self {
            Self::DevelopmentSavedPhoto(source) => source.read_current(work_owner).await,
            Self::ApprovedPortrait(source) | Self::CommittedPortrait(source) => {
                source.read_current(work_owner).await
            }
            Self::AdventureManifest(manifest, key) => {
                if !matches!(work_owner, df_imagery::ImageOwner::Run { .. }) {
                    return Err(super::ImageFailure::Refused);
                }
                let manifest = manifest.clone();
                let key = key.clone();
                tokio::task::spawn_blocking(move || manifest.load(&key))
                    .await
                    .map_err(|_| super::ImageFailure::Interrupted)?
            }
            Self::AuthoredPainting { root, painting } => {
                let path = super::super::home_theme::painting_file(root, painting)
                    .map_err(|_| super::ImageFailure::Refused)?;
                let bytes = tokio::task::spawn_blocking(move || read_file(&path))
                    .await
                    .map_err(|_| super::ImageFailure::Interrupted)?
                    .map_err(|_| super::ImageFailure::Refused)?;
                let format = super::super::image_references::format_of(&bytes)?;
                Ok(ReferenceImage::new(format, bytes))
            }
        }
    }
}

/// Minted only after the established photographic art contract resolves a bounded local file.
#[derive(Clone)]
pub(in crate::gameplay) struct ReviewedReference {
    store: ReferenceStore,
    stem: String,
}

impl std::fmt::Debug for ReviewedReference {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ReviewedReference")
    }
}

impl ReviewedReference {
    pub(in crate::gameplay) fn reload(&self) -> Result<ReferenceImage, ReferenceError> {
        self.store.load(&self.stem)
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
                Ok(bytes) => {
                    let mut reference = ReferenceImage::new(format, bytes);
                    reference.origin = ReferenceOrigin::Reviewed(ReviewedReference {
                        store: self.clone(),
                        stem: photo.to_owned(),
                    });
                    return Ok(reference);
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

/// Only the server-selected adventure manifest can restore its previously issued local aliases.
/// Each read rechecks bounded manifest membership and refuses symlink/non-file cache entries.
#[derive(Clone)]
pub(in crate::gameplay) struct AdventureManifest {
    manifest: PathBuf,
    cache: PathBuf,
}
impl std::fmt::Debug for AdventureManifest {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str("AdventureManifest")
    }
}
impl AdventureManifest {
    pub(in crate::gameplay) fn new(directory: &Path, cache: PathBuf) -> Self {
        Self {
            manifest: directory.join(super::MANIFEST_NAME),
            cache,
        }
    }
    pub(in crate::gameplay) fn load(
        &self,
        key: &super::ImageKey,
    ) -> Result<ReferenceImage, super::ImageFailure> {
        let reader = super::manifest_tail(&self.manifest).ok_or(super::ImageFailure::Refused)?;
        let format = super::manifest_records(reader)
            .find_map(|(found, format)| (found == *key).then_some(format))
            .ok_or(super::ImageFailure::Refused)?;
        for format in super::legacy_stored_formats(format) {
            let path = self.cache.join(super::legacy_file_name(key, *format));
            if let Some(bytes) = super::read_cache_file(&path) {
                return Ok(ReferenceImage::from_trusted(
                    *format,
                    bytes,
                    TrustedReferenceOrigin::AdventureManifest(self.clone(), key.clone()),
                ));
            }
        }
        Err(super::ImageFailure::Refused)
    }
}

/// Minted only from a currently registered canonical Work selected from committed checkpoint data.
pub(in crate::gameplay) trait CommittedPortraitProof: Send + Sync {
    fn authorize(
        &self,
        owner: &df_imagery::ImageOwner,
        asset: &df_model::checkpoint::AssetReference,
    ) -> Result<(), super::ImageFailure>;
}

/// Implemented only at the authenticated development control handler; it rechecks the actual
/// operator credential and reads the configured bounded saved image on every import attempt.
pub(in crate::gameplay) trait DevelopmentSavedPhotoSource: Send + Sync {
    fn read_current<'a>(
        &'a self,
        owner: &'a df_imagery::ImageOwner,
    ) -> super::central::ReferenceFuture<'a, ReferenceImage>;
}
