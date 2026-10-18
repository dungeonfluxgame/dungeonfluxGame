//! Server-side photographic quality gate for generated gameplay images.

use std::time::Duration;

use df_providers::{
    NativePlateJudge, PhotoDress, PhotoFrame, PhotoJudgeKind, PhotoMedium, PhotoPeopleCount,
    PhotoSetting, PhotoText, PlateJudgeError,
};

use super::ImageFormat;

const MAXIMUM_VISION_WIDTH: u32 = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PhotoCheckKind {
    ScenePlate,
    CanonicalRoom,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "canonical characters arrive as transparent cut-outs, which the frame label reads as a composite"
        )
    )]
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
pub(super) enum PhotoCheckFailure {
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
                "The previous result did not look like a real camera photograph; make a real photograph of real people in a real place."
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
pub(super) enum PhotoCheckOutcome {
    Pass,
    Fail(PhotoCheckFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PhotoCheckError {
    ImagePreparation,
    Judge(PlateJudgeError),
}

impl PhotoCheckError {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::ImagePreparation => "image_preparation",
            Self::Judge(error) => error.label(),
        }
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
