//! Server-side photographic quality gate for generated gameplay images.

use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use df_providers::{
    NativePlateJudge, PhotoDress, PhotoFrame, PhotoJudgeKind, PhotoMedium, PhotoPeopleCount,
    PhotoSetting, PhotoText, PlateJudgeError,
};

use super::ImageFormat;

const MAXIMUM_VISION_WIDTH: u32 = 1024;

/// A fixed-size diagnostic ring; rejected bytes never enter the public imagery cache.
const MAXIMUM_REJECTED_CANDIDATES: usize = 64;
static REJECTED_CANDIDATE_SLOT: Mutex<usize> = Mutex::new(0);

/// Retains the exact rejected bytes and generation prompt under the development artifact root.
/// Records the path (or an explicit retention error) on the PHOTO_CHECK failure line.
pub(super) async fn retain_rejected_candidate(
    kind: PhotoCheckKind,
    bytes: &[u8],
    format: ImageFormat,
    failure: PhotoCheckFailure,
    prompt: &str,
) {
    let root = std::env::var_os("DUNGEONFLUX_ARTIFACT_ROOT")
        .map_or_else(|| PathBuf::from("artifacts"), PathBuf::from);
    let bytes = bytes.to_vec();
    let prompt = prompt.to_owned();
    let retained = tokio::task::spawn_blocking(move || {
        retain_candidate_at(&root, kind, &bytes, format, failure, &prompt)
    })
    .await
    .map_err(|error| io::Error::other(format!("photo reject retention task: {error}")))
    .and_then(|retained| retained);
    record_retention(kind, failure, &retained);
}

fn record_retention(
    kind: PhotoCheckKind,
    failure: PhotoCheckFailure,
    retained: &io::Result<PathBuf>,
) -> String {
    let (path, error) = match retained {
        Ok(path) => (path.display().to_string(), String::new()),
        Err(error) => ("unavailable".to_owned(), error.to_string()),
    };
    let line = format!(
        "PHOTO_CHECK kind={} result=fail reason={} path={path:?} retention_error={error:?}",
        kind.label(),
        failure.label()
    );
    crate::gameplay::actor::record_audit_diagnostic(
        "gameplay.imagery.photo_check",
        line.clone(),
        serde_json::Map::from_iter([
            ("kind".to_owned(), serde_json::json!(kind.label())),
            ("result".to_owned(), serde_json::json!("fail")),
            ("reason".to_owned(), serde_json::json!(failure.label())),
            ("retained_path".to_owned(), serde_json::json!(path)),
            ("retention_error".to_owned(), serde_json::json!(error)),
        ]),
        &[
            ("photo_check.kind", kind.label().to_owned()),
            ("photo_check.reason", failure.label().to_owned()),
            ("photo_check.rejected_path", path),
            ("audit.line", line.clone()),
        ],
    );
    line
}

fn retain_candidate_at(
    root: &Path,
    kind: PhotoCheckKind,
    bytes: &[u8],
    format: ImageFormat,
    failure: PhotoCheckFailure,
    prompt: &str,
) -> io::Result<PathBuf> {
    // Serialize ring writes across concurrent image jobs, including their metadata sidecars.
    let mut slot = REJECTED_CANDIDATE_SLOT
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let directory = root.join("photo-check-rejected");
    std::fs::create_dir_all(&directory)?;
    // Fill unused slots first so separate artifact roots do not consume one another's ring.
    let selected = (0..MAXIMUM_REJECTED_CANDIDATES)
        .find(|candidate| {
            !directory
                .join(format!("candidate-{candidate:02}.image"))
                .exists()
        })
        .unwrap_or(*slot);
    let stem = format!("candidate-{selected:02}");
    *slot = (selected + 1) % MAXIMUM_REJECTED_CANDIDATES;
    let path = directory.join(format!("{stem}.image"));
    let metadata = serde_json::json!({
        "kind": kind.label(),
        "reason": failure.label(),
        "correction": failure.correction(),
        "prompt": prompt,
        "format": format!("{format:?}"),
        "created_unix_ms": SystemTime::now().duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?.as_millis(),
        "bytes": bytes.len(),
    });
    std::fs::write(&path, bytes)?;
    std::fs::write(directory.join(format!("{stem}.json")), metadata.to_string())?;
    Ok(path)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PhotoCheckKind {
    ScenePlate,
    CanonicalRoom,
    CanonicalCharacter,
}

impl PhotoCheckKind {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::ScenePlate => "scene_plate",
            Self::CanonicalRoom => "canonical_room",
            Self::CanonicalCharacter => "canonical_character",
        }
    }
}

impl From<PhotoCheckKind> for PhotoJudgeKind {
    fn from(kind: PhotoCheckKind) -> Self {
        match kind {
            PhotoCheckKind::ScenePlate => Self::ScenePlate,
            PhotoCheckKind::CanonicalRoom => Self::CanonicalRoom,
            PhotoCheckKind::CanonicalCharacter => Self::CanonicalCharacter,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PhotoCheckFailure {
    GridOrCollage,
    CutoutComposite,
    NotPhotograph,
    ModernSetting,
    VisibleText,
    ModernClothing,
    PeopleInEmptyRoom,
    CharacterCountNotOne,
    CharacterHasNoPeople,
}

impl PhotoCheckFailure {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::GridOrCollage => "grid_or_collage",
            Self::CutoutComposite => "cutout_composite",
            Self::NotPhotograph => "not_photograph",
            Self::ModernSetting => "modern_setting",
            Self::VisibleText => "visible_text",
            Self::ModernClothing => "modern_clothing",
            Self::PeopleInEmptyRoom => "people_in_empty_room",
            Self::CharacterCountNotOne => "character_count_not_one",
            Self::CharacterHasNoPeople => "character_has_no_people",
        }
    }

    /// What the one corrected regeneration is told about the rejected photograph, in terms of
    /// what the photograph must be; it names no other medium.
    pub(super) const fn correction(self) -> &'static str {
        match self {
            Self::GridOrCollage => {
                "The previous result was split into several pictures; make one single continuous camera frame of one place."
            }
            Self::CutoutComposite => {
                "In the previous result the people looked pasted in; photograph everyone standing in the place itself, in its light and shadow."
            }
            Self::NotPhotograph => {
                "The previous result did not have convincing photographic materials and light; photograph the requested subject with natural material textures, coherent camera perspective and physically consistent light and shadows. Keep the requested people count, including exactly zero people for an empty place. Clean surfaces and even daylight are welcome."
            }
            Self::ModernSetting => {
                "The previous result showed a modern place; show a pre-industrial town of timber, stone, plaster and tile, lit only by daylight, lanterns, candles and fire."
            }
            Self::VisibleText => {
                "The previous result showed lettering; leave every sign, board, cloth and wall surface plain and blank."
            }
            Self::ModernClothing => {
                "In the previous result someone wore modern clothes; dress every person in plain period costume."
            }
            Self::PeopleInEmptyRoom => {
                "The previous result showed people; the place is empty, with exactly zero people."
            }
            Self::CharacterCountNotOne => {
                "The previous result did not show exactly one person; show exactly one."
            }
            Self::CharacterHasNoPeople => {
                "The previous result showed nobody; show exactly one person."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PhotoCheckOutcome {
    Pass,
    Fail(PhotoCheckFailure),
}

impl PhotoCheckOutcome {
    pub(in crate::gameplay) const fn verdict(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail(_) => "fail",
        }
    }

    pub(in crate::gameplay) const fn reason(self) -> Option<&'static str> {
        match self {
            Self::Pass => None,
            Self::Fail(reason) => Some(reason.label()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PhotoCheckError {
    ImagePreparation,
    Judge(PlateJudgeError),
    FileRead,
    FileTooLarge,
    FileTask,
}

impl PhotoCheckError {
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::ImagePreparation => "image_preparation",
            Self::Judge(error) => error.label(),
            Self::FileRead => "file_read",
            Self::FileTooLarge => "file_too_large",
            Self::FileTask => "file_task",
        }
    }
}

impl super::Imagery {
    /// Reuses the publication gate for a saved image, without generating or caching anything.
    pub(in crate::gameplay) async fn check_saved_photo(
        judge: &NativePlateJudge,
        kind: PhotoJudgeKind,
        path: &Path,
    ) -> Result<PhotoCheckOutcome, PhotoCheckError> {
        let path = path.to_owned();
        let bytes = tokio::task::spawn_blocking(move || {
            const MAXIMUM_FILE_BYTES: u64 = 16 * 1024 * 1024;
            let file = std::fs::File::open(path).map_err(|_| PhotoCheckError::FileRead)?;
            let mut bytes = Vec::new();
            file.take(MAXIMUM_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| PhotoCheckError::FileRead)?;
            if bytes.len() as u64 > MAXIMUM_FILE_BYTES {
                return Err(PhotoCheckError::FileTooLarge);
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| PhotoCheckError::FileTask)??;
        let format = match image::guess_format(&bytes) {
            Ok(image::ImageFormat::Jpeg) => ImageFormat::Jpeg,
            Ok(image::ImageFormat::Png) => ImageFormat::Png,
            Ok(image::ImageFormat::WebP) => ImageFormat::Webp,
            _ => return Err(PhotoCheckError::ImagePreparation),
        };
        let kind = match kind {
            PhotoJudgeKind::CanonicalRoom => PhotoCheckKind::CanonicalRoom,
            PhotoJudgeKind::ScenePlate => PhotoCheckKind::ScenePlate,
            PhotoJudgeKind::CanonicalCharacter => PhotoCheckKind::CanonicalCharacter,
        };
        check_photo(judge, kind, &bytes, format, super::PHOTO_CHECK_DEADLINE).await
    }
}

/// Checks one generated image before publication. Provider failures remain typed errors; a
/// returned `Fail` names the visible defect so the caller can make one corrected regeneration.
pub(super) async fn check_photo(
    judge: &NativePlateJudge,
    kind: PhotoCheckKind,
    bytes: &[u8],
    format: ImageFormat,
    deadline: Duration,
) -> Result<PhotoCheckOutcome, PhotoCheckError> {
    let picture = super::vision_picture(bytes, format, MAXIMUM_VISION_WIDTH)
        .map_err(|_| PhotoCheckError::ImagePreparation)?;
    let answer = judge
        .judge_photo_quality(&picture, kind.into(), deadline)
        .await
        .map_err(PhotoCheckError::Judge)?;
    Ok(classify(
        kind,
        answer.frame,
        answer.medium,
        answer.text,
        answer.dress,
        answer.setting,
        answer.people_count,
        answer.people_present,
    ))
}

fn classify(
    kind: PhotoCheckKind,
    frame: PhotoFrame,
    medium: PhotoMedium,
    text: PhotoText,
    dress: PhotoDress,
    setting: PhotoSetting,
    people_count: Option<PhotoPeopleCount>,
    people_present: Option<bool>,
) -> PhotoCheckOutcome {
    let failure = match frame {
        PhotoFrame::GridOrCollage => Some(PhotoCheckFailure::GridOrCollage),
        PhotoFrame::CutoutComposite => Some(PhotoCheckFailure::CutoutComposite),
        PhotoFrame::SinglePhotograph => None,
    }
    .or_else(|| match medium {
        PhotoMedium::Photograph => None,
        PhotoMedium::PaintingOrIllustration | PhotoMedium::CgiOrRender => {
            Some(PhotoCheckFailure::NotPhotograph)
        }
    })
    .or_else(|| match setting {
        PhotoSetting::Period => None,
        PhotoSetting::Modern => Some(PhotoCheckFailure::ModernSetting),
    })
    .or_else(|| match text {
        PhotoText::None => None,
        PhotoText::VisibleText => Some(PhotoCheckFailure::VisibleText),
    })
    .or_else(|| match (kind, people_present) {
        (PhotoCheckKind::CanonicalRoom, Some(false)) => None,
        (PhotoCheckKind::CanonicalRoom, Some(true)) => Some(PhotoCheckFailure::PeopleInEmptyRoom),
        (PhotoCheckKind::CanonicalRoom, None) => Some(PhotoCheckFailure::PeopleInEmptyRoom),
        _ => None,
    })
    .or_else(|| match dress {
        PhotoDress::ModernClothing => Some(PhotoCheckFailure::ModernClothing),
        PhotoDress::Period | PhotoDress::NoPeople => None,
    })
    .or_else(|| match (kind, dress, people_count) {
        (PhotoCheckKind::CanonicalCharacter, PhotoDress::NoPeople, _) => {
            Some(PhotoCheckFailure::CharacterHasNoPeople)
        }
        (PhotoCheckKind::CanonicalCharacter, _, Some(PhotoPeopleCount::ExactlyOne)) => None,
        (PhotoCheckKind::CanonicalCharacter, _, Some(PhotoPeopleCount::NotOne)) => {
            Some(PhotoCheckFailure::CharacterCountNotOne)
        }
        (PhotoCheckKind::CanonicalCharacter, _, None) => {
            Some(PhotoCheckFailure::CharacterCountNotOne)
        }
        (PhotoCheckKind::CanonicalRoom, _, _) => None,
        (PhotoCheckKind::ScenePlate, _, _) => None,
    });
    failure.map_or(PhotoCheckOutcome::Pass, PhotoCheckOutcome::Fail)
}

#[cfg(test)]
#[path = "photo_check_tests.rs"]
mod tests;
