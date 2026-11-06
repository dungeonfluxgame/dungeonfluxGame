//! Non-raster analysis requests and audience-safe typed results; no asset is fabricated.
use super::types::OperationHandle;
use crate::{
    Audience, ExecutionMode, Freshness, OperationIdentity, PrivateImageText, PurposedUse,
    SourceRef, Urgency,
};

/// One analysis of an existing source, recorded and replayed like an image request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageAnalysisRequest {
    pub identity: OperationIdentity,
    pub purpose: PurposedUse,
    pub operation: ImageAnalysisOperation,
    pub execution: ExecutionMode,
    pub freshness: Freshness,
    pub urgency: Urgency,
    pub audience: Audience,
    pub expected_source_revision: u64,
}
/// The closed set of analyses; each names its source and its dependencies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageAnalysisOperation {
    PhotoInspection(PhotoInspectionSpec),
    HeadLocations(HeadLocationsSpec),
    Segmentation(SegmentationSpec),
    StagingIdentity(StagingIdentitySpec),
    StagingFacing(StagingFacingSpec),
}
/// One named thing to look for, described in private text that never reaches logs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisTarget {
    pub id: PrivateImageText,
    pub description: PrivateImageText,
}
/// A reference picture of one named person for identity matching.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisReference {
    pub id: PrivateImageText,
    pub source: SourceRef,
}
/// Locate the heads of named targets in one source, optionally guided by a face picture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadLocationsSpec {
    pub source: SourceRef,
    pub face: Option<SourceRef>,
    pub targets: Vec<AnalysisTarget>,
    pub contract_revision: u32,
}
/// The one noun a segmentation looks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentationPrompt {
    Person,
    Head,
}
impl SegmentationPrompt {
    pub fn label(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Head => "head",
        }
    }
}
/// How the source is prepared before segmentation: a figure cut-out or a whole plate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentationPreparation {
    FigureMask,
    ScenePlate,
}
/// Segment one source into instance masks, one mask set per prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentationSpec {
    pub preparation: SegmentationPreparation,
    pub source: SourceRef,
    pub prompts: Vec<SegmentationPrompt>,
    pub maximum_masks: u32,
    pub contract_revision: u32,
}
/// Match the instances of a finished segmentation to reference pictures of named people.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagingIdentitySpec {
    pub source: SourceRef,
    pub segmentation: OperationHandle,
    pub locations: Option<OperationHandle>,
    pub references: Vec<AnalysisReference>,
    pub contract_revision: u32,
}
/// Judge which way each identified instance of a segmentation faces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagingFacingSpec {
    pub source: SourceRef,
    pub segmentation: OperationHandle,
    pub identity: OperationHandle,
    pub contract_revision: u32,
}

/// Finite unit-interval values retain exact source float bits; no geometry quantization is
/// introduced.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AnalysisUnit(u32);
impl AnalysisUnit {
    pub fn new(value: f32) -> Result<Self, AnalysisValueError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(AnalysisValueError::Coordinate);
        }
        Ok(Self(if value == 0.0 { 0 } else { value.to_bits() }))
    }
    pub fn get(self) -> f32 {
        f32::from_bits(self.0)
    }
}
impl std::fmt::Debug for AnalysisUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}
/// Why an analysis value was refused: a coordinate, a box or a mask out of contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisValueError {
    Coordinate,
    Box,
    Mask,
}
/// A box in unit coordinates of its picture: `left < right` and `top < bottom`, each 0 to 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NormalizedImageBox {
    left: AnalysisUnit,
    top: AnalysisUnit,
    right: AnalysisUnit,
    bottom: AnalysisUnit,
}
impl NormalizedImageBox {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Result<Self, AnalysisValueError> {
        if left >= right || top >= bottom {
            return Err(AnalysisValueError::Box);
        }
        Ok(Self {
            left: AnalysisUnit::new(left)?,
            top: AnalysisUnit::new(top)?,
            right: AnalysisUnit::new(right)?,
            bottom: AnalysisUnit::new(bottom)?,
        })
    }
    pub fn coordinates(self) -> [f32; 4] {
        [
            self.left.get(),
            self.top.get(),
            self.right.get(),
            self.bottom.get(),
        ]
    }
}
/// The head box of one named target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisLocatedHead {
    pub id: PrivateImageText,
    pub head: NormalizedImageBox,
}
/// A run of `length` mask pixels from zero-based row-major pixel `start`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnalysisMaskRun {
    pub start: u32,
    pub length: u32,
}
/// One instance's mask: its supplier index, score and ordered, non-overlapping runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisMask {
    pub index: u32,
    pub score: AnalysisUnit,
    pub runs: Vec<AnalysisMaskRun>,
}
/// The instance masks one prompt found, over a picture of `width_px` by `height_px`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisMaskSet {
    pub prompt: SegmentationPrompt,
    pub width_px: u32,
    pub height_px: u32,
    pub instances: Vec<AnalysisMask>,
}
impl AnalysisMaskSet {
    /// Checks normalized zero-based runs against the exact submitted picture dimensions.
    pub fn validate(&self) -> Result<(), AnalysisValueError> {
        if self.width_px == 0 || self.height_px == 0 {
            return Err(AnalysisValueError::Mask);
        }
        let pixels = u64::from(self.width_px) * u64::from(self.height_px);
        let mut indices = std::collections::BTreeSet::new();
        for instance in &self.instances {
            if !indices.insert(instance.index) {
                return Err(AnalysisValueError::Mask);
            }
            let mut end = 0u64;
            for run in &instance.runs {
                let start = u64::from(run.start);
                let next = start + u64::from(run.length);
                if run.length == 0 || start < end || next > pixels {
                    return Err(AnalysisValueError::Mask);
                }
                end = next;
            }
        }
        Ok(())
    }
}
/// The candidate instance (numbered from 1) matched to one reference, or none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisIdentityMatch {
    pub reference_id: PrivateImageText,
    pub candidate: Option<u32>,
}
/// Which way an instance faces in the picture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisFacingDirection {
    Left,
    Right,
    TowardCamera,
    AwayFromCamera,
}
/// The facing of one candidate instance, or none when it could not be judged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnalysisFacing {
    pub candidate: u32,
    pub facing: Option<AnalysisFacingDirection>,
}

/// These are observations over an existing source, never catalog image publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageAnalysisResult {
    PhotoInspection {
        source: SourceRef,
        assessment: PhotoInspectionAssessment,
    },
    HeadLocations {
        source: SourceRef,
        contract_revision: u32,
        heads: Vec<AnalysisLocatedHead>,
    },
    Segmentation {
        source: SourceRef,
        contract_revision: u32,
        masks: Vec<AnalysisMaskSet>,
    },
    StagingIdentity {
        source: SourceRef,
        contract_revision: u32,
        matches: Vec<AnalysisIdentityMatch>,
    },
    StagingFacing {
        source: SourceRef,
        contract_revision: u32,
        facings: Vec<AnalysisFacing>,
    },
}

/// Stored analytical authority facts decoded from the immutable canonical envelope.
#[derive(Clone, Copy, Debug)]
pub struct StoredAnalysisAuthorization<'a> {
    pub handle: &'a OperationHandle,
    pub owner: &'a crate::ImageOwner,
    pub purpose: PurposedUse,
    pub operation: &'a ImageAnalysisOperation,
    pub execution: ExecutionMode,
    pub freshness: Freshness,
    pub audience: &'a Audience,
    pub expected_source_revision: u64,
}
/// An authorized, finally selected analysis result and the job revision it was read at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageAnalysisObservation {
    pub job_revision: u64,
    /// Actual durable final selection time; read time does not expire a timely result.
    pub selected_at_ms: i64,
    pub result: ImageAnalysisResult,
}

/// Inspect one generated photograph against references under a closed inspection policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhotoInspectionSpec {
    pub source: SourceRef,
    pub references: Vec<SourceRef>,
    pub inspection: crate::ImageInspection,
}
/// The closed photographic defects an inspection can find.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhotoInspectionFailure {
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
/// Why an inspection gave no judgement; an unjudged photograph is never a pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhotoInspectionUnjudgedReason {
    Preparation,
    Transport,
    Provider,
    InvalidResponse,
    Deadline,
    Disabled,
    Cancelled,
}
/// The outcome of one photo inspection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhotoInspectionVerdict {
    Pass,
    Failed(PhotoInspectionFailure),
    Unjudged(PhotoInspectionUnjudgedReason),
}
/// One visible feature the inspection recorded, with its region in permille of the picture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhotoVisibleFeature {
    pub name: PrivateImageText,
    pub look: PrivateImageText,
    pub region: [u16; 4],
    pub recorded_id: Option<PrivateImageText>,
}
/// The place and source hash the recorded features belong to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhotoFeatureBinding {
    pub place: PrivateImageText,
    pub source_hash: PrivateImageText,
}
/// A photo inspection's verdict and the visible features it recorded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhotoInspectionAssessment {
    pub contract_revision: u32,
    pub verdict: PhotoInspectionVerdict,
    pub visible_features: Vec<PhotoVisibleFeature>,
    pub source_binding: Option<PhotoFeatureBinding>,
}
impl ImageAnalysisOperation {
    pub fn sources(&self) -> Vec<&SourceRef> {
        match self {
            Self::PhotoInspection(spec) => std::iter::once(&spec.source)
                .chain(spec.references.iter())
                .collect(),
            Self::HeadLocations(spec) => std::iter::once(&spec.source)
                .chain(spec.face.iter())
                .collect(),
            Self::Segmentation(spec) => vec![&spec.source],
            Self::StagingIdentity(spec) => std::iter::once(&spec.source)
                .chain(spec.references.iter().map(|reference| &reference.source))
                .collect(),
            Self::StagingFacing(spec) => vec![&spec.source],
        }
    }
    pub fn dependencies(&self) -> Vec<&OperationHandle> {
        match self {
            Self::StagingIdentity(spec) => std::iter::once(&spec.segmentation)
                .chain(spec.locations.iter())
                .collect(),
            Self::StagingFacing(spec) => vec![&spec.segmentation, &spec.identity],
            _ => Vec::new(),
        }
    }
}
