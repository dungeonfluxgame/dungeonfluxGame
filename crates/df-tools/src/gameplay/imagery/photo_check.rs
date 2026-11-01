//! Development saved-photo inspections and compatibility verdicts through ImageService.

use std::{io::Read, path::Path};

use df_providers::PhotoJudgeKind;

use super::{ImageFormat, ReferenceImage};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PhotoCheckKind {
    ScenePlate,
    CanonicalRoom,
    CanonicalCharacter,
}

#[cfg(test)]
impl PhotoCheckKind {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::ScenePlate => "scene_plate",
            Self::CanonicalRoom => "canonical_room",
            Self::CanonicalCharacter => "canonical_character",
        }
    }
}

#[cfg(test)]
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
    HeroCount { expected: u8, seen: u64 },
    UnexpectedDrawnWeapon,
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
            Self::HeroCount { .. } => "hero_count",
            Self::UnexpectedDrawnWeapon => "unexpected_drawn_weapon",
        }
    }

    /// What the one corrected regeneration is told about the rejected photograph, in terms of
    /// what the photograph must be; it names no other medium.
    #[cfg(test)]
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
            Self::UnexpectedDrawnWeapon => {
                "Match each person's recorded hand custody and positions exactly. Empty hands stay empty; weapons stay sheathed at the hip or slung on the back, bows unstrung on the shoulder. Show a drawn weapon only for the person whose recorded state explicitly places it drawn in a hand. Reference poses never authorize a drawn weapon."
            }
            Self::HeroCount { .. } => {
                "The previous result showed the wrong number of player heroes. Show exactly the requested number, each once and matching their reference-sheet photograph; include no other figure dressed or posed as a hero."
            }
        }
    }
}

impl From<df_imagery::PhotoInspectionFailure> for PhotoCheckFailure {
    fn from(failure: df_imagery::PhotoInspectionFailure) -> Self {
        match failure {
            df_imagery::PhotoInspectionFailure::GridOrCollage => Self::GridOrCollage,
            df_imagery::PhotoInspectionFailure::CutoutComposite => Self::CutoutComposite,
            df_imagery::PhotoInspectionFailure::NotPhotograph => Self::NotPhotograph,
            df_imagery::PhotoInspectionFailure::ModernSetting => Self::ModernSetting,
            df_imagery::PhotoInspectionFailure::VisibleText => Self::VisibleText,
            df_imagery::PhotoInspectionFailure::ModernClothing => Self::ModernClothing,
            df_imagery::PhotoInspectionFailure::PeopleInEmptyRoom => Self::PeopleInEmptyRoom,
            df_imagery::PhotoInspectionFailure::CharacterCountNotOne => Self::CharacterCountNotOne,
            df_imagery::PhotoInspectionFailure::CharacterHasNoPeople => Self::CharacterHasNoPeople,
            df_imagery::PhotoInspectionFailure::UnexpectedDrawnWeapon => {
                Self::UnexpectedDrawnWeapon
            }
            df_imagery::PhotoInspectionFailure::HeroCount { expected, seen } => {
                Self::HeroCount { expected, seen }
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
    Analysis(super::ImageFailure),
    Unjudged(df_imagery::PhotoInspectionUnjudgedReason),
    FileRead,
    FileTooLarge,
    FileTask,
}

impl PhotoCheckError {
    pub(in crate::gameplay) fn label(self) -> &'static str {
        match self {
            Self::ImagePreparation => "image_preparation",
            Self::Analysis(error) => error.label(),
            Self::Unjudged(reason) => match reason {
                df_imagery::PhotoInspectionUnjudgedReason::Preparation => "image_preparation",
                df_imagery::PhotoInspectionUnjudgedReason::Transport => "transport",
                df_imagery::PhotoInspectionUnjudgedReason::Provider => "provider",
                df_imagery::PhotoInspectionUnjudgedReason::InvalidResponse => "invalid_response",
                df_imagery::PhotoInspectionUnjudgedReason::Deadline => "deadline",
                df_imagery::PhotoInspectionUnjudgedReason::Disabled => "off",
                df_imagery::PhotoInspectionUnjudgedReason::Cancelled => "cancelled",
            },
            Self::FileRead => "file_read",
            Self::FileTooLarge => "file_too_large",
            Self::FileTask => "file_task",
        }
    }
}

impl super::Imagery {
    /// Reads a saved input under the host-reviewed bound, preserving the exact source bytes.
    pub(in crate::gameplay) async fn read_saved_photo(
        path: &Path,
        maximum_bytes: u64,
    ) -> Result<ReferenceImage, PhotoCheckError> {
        let path = path.to_owned();
        let bytes = tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(path).map_err(|_| PhotoCheckError::FileRead)?;
            let metadata = file.metadata().map_err(|_| PhotoCheckError::FileRead)?;
            if !metadata.is_file() {
                return Err(PhotoCheckError::FileRead);
            }
            if maximum_bytes == 0 || metadata.len() > maximum_bytes {
                return Err(PhotoCheckError::FileTooLarge);
            }
            // At u64::MAX no larger byte count is representable; every addressable Vec is smaller.
            let limit = maximum_bytes.saturating_add(1);
            let mut bytes = Vec::new();
            file.take(limit)
                .read_to_end(&mut bytes)
                .map_err(|_| PhotoCheckError::FileRead)?;
            if bytes.len() as u64 > maximum_bytes {
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
        Ok(ReferenceImage::new(format, bytes.into()))
    }

    /// One explicitly fresh durable inspection of an authenticated development source.
    pub(in crate::gameplay) async fn check_saved_photo(
        &self,
        kind: PhotoJudgeKind,
        path: &Path,
        proof: std::sync::Arc<dyn super::DevelopmentSavedPhotoSource>,
    ) -> Result<PhotoCheckOutcome, PhotoCheckError> {
        use df_imagery::{
            Freshness, ImageAnalysisOperation, ImageAnalysisResult, ImageInspection,
            InspectionKind, InspectionRequirement, PhotoInspectionSpec, PhotoInspectionVerdict,
            PrivateImageText,
        };
        let limit = self
            .reviewed_input_limit()
            .map_err(PhotoCheckError::Analysis)?;
        let reference = Self::read_saved_photo(path, limit).await?;
        let context = self
            .consumer_context()
            .await
            .map_err(PhotoCheckError::Analysis)?;
        let route = context
            .config
            .analysis
            .inspection
            .clone()
            .ok_or(PhotoCheckError::Analysis(super::ImageFailure::Off))?;
        let reference = ReferenceImage::from_trusted(
            reference.format,
            reference.bytes,
            super::TrustedReferenceOrigin::DevelopmentSavedPhoto(proof),
        );
        let source = self
            .pin_reference(&reference)
            .await
            .map_err(PhotoCheckError::Analysis)?;
        let inspection = ImageInspection {
            route,
            requirement: InspectionRequirement::Required,
            contract_revision: 1,
            kind: match kind {
                PhotoJudgeKind::CanonicalRoom => InspectionKind::CanonicalRoom,
                PhotoJudgeKind::ScenePlate => InspectionKind::ScenePlate,
                PhotoJudgeKind::CanonicalCharacter => InspectionKind::CanonicalCharacter,
            },
            expected_heroes: None,
            party_reference: None,
            hand_custody: PrivateImageText::new(""),
            staging: Vec::new(),
            maximum_corrections: 0,
            allow_visible_text: false,
        };
        let operation = ImageAnalysisOperation::PhotoInspection(PhotoInspectionSpec {
            source,
            references: Vec::new(),
            inspection,
        });
        let mut nonce = [0; 16];
        crate::gameplay::model::fill_os_entropy(&mut nonce)
            .map_err(|_| PhotoCheckError::Analysis(super::ImageFailure::Interrupted))?;
        let nonce = df_types::OperationId::from_bytes(&nonce)
            .map_err(|_| PhotoCheckError::Analysis(super::ImageFailure::Interrupted))?;
        let observed = super::analysis::execute_with_freshness(
            self,
            "dev-photo-check",
            operation,
            Freshness::Regenerate { nonce },
        )
        .await
        .map_err(PhotoCheckError::Analysis)?;
        let ImageAnalysisResult::PhotoInspection { assessment, .. } = observed.value else {
            return Err(PhotoCheckError::Analysis(super::ImageFailure::BadOutput));
        };
        match assessment.verdict {
            PhotoInspectionVerdict::Pass => Ok(PhotoCheckOutcome::Pass),
            PhotoInspectionVerdict::Failed(failure) => Ok(PhotoCheckOutcome::Fail(failure.into())),
            PhotoInspectionVerdict::Unjudged(reason) => Err(PhotoCheckError::Unjudged(reason)),
        }
    }
}

#[cfg(test)]
#[path = "photo_check_tests.rs"]
mod tests;
