//! Source-bound presentation projections only. Supplier results and paid execution stay in PG.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::PathBuf,
};

#[derive(Clone, Copy)]
pub(in crate::gameplay) enum PresentationAnnotation {
    Layout,
    Figures,
}
impl PresentationAnnotation {
    fn suffix(self) -> &'static str {
        match self {
            Self::Layout => "layout-v1",
            Self::Figures => "figures-v1",
        }
    }
}
pub(in crate::gameplay) struct AnnotationStore {
    root: PathBuf,
    maximum_bytes: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Annotation {
    source: [u8; 32],
    payload: Vec<u8>,
}
impl AnnotationStore {
    pub(in crate::gameplay) fn new(
        root: PathBuf,
        maximum_bytes: u64,
    ) -> Result<Self, std::io::Error> {
        if maximum_bytes == 0 {
            return Err(std::io::Error::other("annotation bound invalid"));
        }
        std::fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        Ok(Self {
            root,
            maximum_bytes,
        })
    }
    fn path(
        &self,
        owner: &df_imagery::ImageOwner,
        key: &super::super::imagery::ImageKey,
        kind: PresentationAnnotation,
    ) -> PathBuf {
        let owner =
            super::super::journey::hex(&Sha256::digest(df_imagery::owner_scope_bytes(owner)));
        self.root
            .join(format!("{owner}-{}-{}.json", key.as_str(), kind.suffix()))
    }
    pub(in crate::gameplay) fn read(
        &self,
        owner: &df_imagery::ImageOwner,
        key: &super::super::imagery::ImageKey,
        kind: PresentationAnnotation,
        source: [u8; 32],
    ) -> Result<Option<Vec<u8>>, std::io::Error> {
        let path = self.path(owner, key, kind);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let encoded_bound = self
            .maximum_bytes
            .checked_mul(4)
            .and_then(|bytes| bytes.checked_add(256))
            .ok_or_else(|| std::io::Error::other("annotation bound overflow"))?;
        if !metadata.is_file() || metadata.len() > encoded_bound {
            return Err(std::io::Error::other("annotation file refused"));
        }
        let file = open_annotation(&path)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("annotation file refused"));
        }
        let mut bytes = Vec::new();
        file.take(encoded_bound + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > encoded_bound {
            return Err(std::io::Error::other("annotation too large"));
        }
        let annotation: Annotation =
            serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        if annotation.payload.len() as u64 > self.maximum_bytes || annotation.source != source {
            return Ok(None);
        }
        Ok(Some(annotation.payload))
    }
    pub(in crate::gameplay) fn write(
        &self,
        owner: &df_imagery::ImageOwner,
        key: &super::super::imagery::ImageKey,
        kind: PresentationAnnotation,
        source: [u8; 32],
        payload: Vec<u8>,
    ) -> Result<(), std::io::Error> {
        if payload.len() as u64 > self.maximum_bytes {
            return Err(std::io::Error::other("annotation too large"));
        }
        let target = self.path(owner, key, kind);
        let mut nonce = [0; 16];
        getrandom::getrandom(&mut nonce)
            .map_err(|_| std::io::Error::other("annotation identity unavailable"))?;
        let staging = self
            .root
            .join(format!(".{}.tmp", super::super::journey::hex(&nonce)));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)?;
            serde_json::to_writer(&mut file, &Annotation { source, payload })
                .map_err(std::io::Error::other)?;
            file.flush()?;
            file.sync_all()?;
            std::fs::rename(&staging, &target)
        })();
        if result.is_err() {
            let _removed = std::fs::remove_file(staging);
        }
        result
    }
}
pub(in crate::gameplay) fn source_binding(
    receipt: &df_imagery::runtime::SourceReceipt,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"df-image-presentation-source/1");
    digest.update(receipt.pin.asset.as_bytes());
    digest.update(receipt.pin.digest.as_bytes());
    digest.update(receipt.attempt.as_bytes());
    digest.update(receipt.write.as_bytes());
    digest.update(receipt.version.as_str().as_bytes());
    digest.update(receipt.suppression_generation.to_be_bytes());
    digest.update([match receipt.retention {
        df_assets::RetentionClass::Quarantine => 0,
        df_assets::RetentionClass::Intermediate => 1,
        df_assets::RetentionClass::Personal => 2,
        df_assets::RetentionClass::CampaignMedia => 3,
        df_assets::RetentionClass::Export => 4,
        df_assets::RetentionClass::Backup => 5,
    }]);
    digest.finalize().into()
}

fn open_annotation(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64")))]
        const FLAGS: i32 = 0x20000 | 0x800;
        #[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
        const FLAGS: i32 = 0x8000 | 0x800;
        #[cfg(any(target_os = "macos", target_os = "freebsd"))]
        const FLAGS: i32 = 0x100 | 0x4;
        #[cfg(not(any(
            all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64")),
            all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")),
            target_os = "macos",
            target_os = "freebsd"
        )))]
        const FLAGS: i32 = 0;
        options.custom_flags(FLAGS);
    }
    options.open(path)
}
