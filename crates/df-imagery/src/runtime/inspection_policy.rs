//! Pure compatibility inspection policy over a retained supplier judgement.
//!
//! No networking, retries, corrections, storage or publication occur here.
//! Malformed/unavailable judgements remain unjudged at the runtime owner;
//! only closed observed labels determine a photographic defect.

use crate::backends::analysis::photo::{
    PhotoDress, PhotoFrame, PhotoMedium, PhotoPeopleCount, PhotoSetting, PhotoText,
    parse_photo_answer,
};
use crate::request::{ImageInspection, InspectionKind};
use serde_json::{Value, json};

/// Bound preserved from the existing scene photograph evidence contract.
pub(crate) const MAXIMUM_PICTURE_FEATURES: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InspectionError {
    InvalidRequest,
    InvalidAnswer,
}

impl std::fmt::Display for InspectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest => formatter.write_str("the inspection request is invalid"),
            Self::InvalidAnswer => formatter.write_str("the inspection answer is invalid"),
        }
    }
}

impl std::error::Error for InspectionError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PhotoCheckCause {
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

impl PhotoCheckCause {
    pub(crate) const fn key(self) -> &'static str {
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
    pub(crate) const fn correction(self) -> &'static str {
        match self {
            Self::GridOrCollage => concat!(
                "The previous result was split into several pictures; make one single continuous ",
                "camera frame of one place."
            ),
            Self::CutoutComposite => concat!(
                "In the previous result the people looked pasted in; photograph everyone standing ",
                "in the place itself, in its light and shadow."
            ),
            Self::NotPhotograph => concat!(
                "The previous result did not have convincing photographic materials and light; ",
                "photograph the requested subject with natural material textures, coherent camera ",
                "perspective and physically consistent light and shadows. ",
                "Keep the requested people count, including exactly zero people for an empty ",
                "place. ",
                "Clean surfaces and even daylight are welcome."
            ),
            Self::ModernSetting => concat!(
                "The previous result showed a modern place; show a pre-industrial town of timber, ",
                "stone, plaster and tile, lit only by daylight, lanterns, candles and fire."
            ),
            Self::VisibleText => concat!(
                "The previous result showed lettering; leave every sign, board, cloth and wall ",
                "surface plain and blank."
            ),
            Self::ModernClothing => concat!(
                "In the previous result someone wore modern clothes; dress every person in plain ",
                "period costume."
            ),
            Self::PeopleInEmptyRoom => {
                "The previous result showed people; the place is empty, with exactly zero people."
            }
            Self::CharacterCountNotOne => {
                "The previous result did not show exactly one person; show exactly one."
            }
            Self::CharacterHasNoPeople => {
                "The previous result showed nobody; show exactly one person."
            }
            Self::UnexpectedDrawnWeapon => concat!(
                "Match each person's recorded hand custody and positions exactly. ",
                "Empty hands stay empty; weapons stay sheathed at the hip or slung on the back, ",
                "bows unstrung on the shoulder. ",
                "Show a drawn weapon only for the person whose recorded state explicitly places ",
                "it drawn in a hand. ",
                "Reference poses never authorize a drawn weapon."
            ),
            Self::HeroCount { .. } => concat!(
                "The previous result showed the wrong number of player heroes. ",
                "Show exactly the requested number, each once and matching their reference-sheet ",
                "photograph; include no other figure dressed or posed as a hero."
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnjudgedReason {
    Preparation,
    Transport,
    Provider,
    InvalidResponse,
    Deadline,
    Disabled,
    Cancelled,
}

impl UnjudgedReason {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Preparation => "preparation",
            Self::Transport => "transport",
            Self::Provider => "provider",
            Self::InvalidResponse => "invalid_response",
            Self::Deadline => "deadline",
            Self::Disabled => "disabled",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InspectionVerdict {
    Pass,
    Failed(PhotoCheckCause),
    Unjudged(UnjudgedReason),
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct VisibleFeature {
    pub(crate) name: String,
    pub(crate) look: String,
    pub(crate) region: [u16; 4],
    pub(crate) recorded_id: Option<String>,
}
impl std::fmt::Debug for VisibleFeature {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VisibleFeature")
            .field("region", &self.region)
            .field("has_recorded_id", &self.recorded_id.is_some())
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct FeatureBinding {
    pub(crate) place: String,
    pub(crate) source_hash: String,
}
impl std::fmt::Debug for FeatureBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FeatureBinding")
            .finish_non_exhaustive()
    }
}

/// The policy reading stays bound to the retained candidate by the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InspectionAssessment {
    pub(crate) contract_revision: u32,
    pub(crate) verdict: InspectionVerdict,
    pub(crate) visible_features: Vec<VisibleFeature>,
    pub(crate) source_binding: Option<FeatureBinding>,
}

/// Retains the existing frozen v1 count/reference and one-correction contract.
pub(crate) fn validate_spec(spec: &ImageInspection) -> Result<(), InspectionError> {
    if spec.contract_revision == 0
        || spec.maximum_corrections > 1
        || !matches!(
            (spec.expected_heroes, spec.party_reference),
            (None, None) | (Some(1..=6), Some(1..))
        )
    {
        return Err(InspectionError::InvalidRequest);
    }
    Ok(())
}

/// Interprets provider JSON, preserving count/custody/quality failure precedence.
pub(crate) fn interpret(
    answer: Value,
    spec: &ImageInspection,
) -> Result<InspectionAssessment, InspectionError> {
    validate_spec(spec)?;
    let scene = spec.kind == InspectionKind::ScenePlate && spec.expected_heroes.is_some();
    let verdict = if scene {
        parse_scene_photo_answer(&answer, spec)?
    } else {
        let quality = parse_photo_answer(&answer, spec.kind)?;
        classify(PhotoCheckJudgement {
            kind: spec.kind,
            frame: quality.frame,
            medium: quality.medium,
            text: quality.text,
            dress: quality.dress,
            setting: quality.setting,
            people_count: quality.people_count,
            people_present: quality.people_present,
        })
    };
    let visible_features = if verdict == InspectionVerdict::Pass && scene {
        parse_features(
            answer
                .get("visible_features")
                .ok_or(InspectionError::InvalidAnswer)?,
            spec,
        )?
    } else {
        Vec::new()
    };
    let source_binding = binding(spec, !visible_features.is_empty());
    Ok(InspectionAssessment {
        contract_revision: spec.contract_revision,
        verdict,
        visible_features,
        source_binding,
    })
}

fn binding(spec: &ImageInspection, has_features: bool) -> Option<FeatureBinding> {
    if !has_features {
        return None;
    }
    let context: Value = serde_json::from_str(spec.hand_custody.text()).ok()?;
    Some(FeatureBinding {
        place: context.get("place")?.as_str()?.to_owned(),
        source_hash: context.get("source_hash")?.as_str()?.to_owned(),
    })
}

fn bounded(text: &str, maximum: usize) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= maximum
        && !text.chars().any(char::is_control)
}

fn parse_features(
    value: &Value,
    spec: &ImageInspection,
) -> Result<Vec<VisibleFeature>, InspectionError> {
    let entries = value
        .as_array()
        .filter(|entries| entries.len() <= MAXIMUM_PICTURE_FEATURES)
        .ok_or(InspectionError::InvalidAnswer)?;
    let context = serde_json::from_str::<Value>(spec.hand_custody.text()).ok();
    let ids = context
        .as_ref()
        .and_then(|context| context.get("public_records"))
        .and_then(Value::as_array)
        .map(|records| {
            records
                .iter()
                .filter_map(|record| record.get("id").and_then(Value::as_str))
                .collect::<std::collections::BTreeSet<_>>()
        });
    let mut features = Vec::with_capacity(entries.len());
    for entry in entries {
        let fields = entry
            .as_object()
            .filter(|fields| {
                fields.len() == 4
                    && ["name", "look", "region", "recorded_id"]
                        .iter()
                        .all(|key| fields.contains_key(*key))
            })
            .ok_or(InspectionError::InvalidAnswer)?;
        let name = fields
            .get("name")
            .and_then(Value::as_str)
            .filter(|text| bounded(text, 40) && text.chars().any(char::is_alphanumeric))
            .ok_or(InspectionError::InvalidAnswer)?;
        let look = fields
            .get("look")
            .and_then(Value::as_str)
            .filter(|text| bounded(text, 120))
            .ok_or(InspectionError::InvalidAnswer)?;
        let coords = fields
            .get("region")
            .and_then(Value::as_array)
            .filter(|region| region.len() == 4)
            .ok_or(InspectionError::InvalidAnswer)?;
        let mut region = [0u16; 4];
        for (output, coord) in region.iter_mut().zip(coords) {
            *output = coord
                .as_u64()
                .filter(|coord| *coord <= 1000)
                .and_then(|coord| u16::try_from(coord).ok())
                .ok_or(InspectionError::InvalidAnswer)?;
        }
        if region[0] >= region[2] || region[1] >= region[3] {
            return Err(InspectionError::InvalidAnswer);
        }
        let recorded_id = match fields.get("recorded_id") {
            Some(Value::Null) => None,
            Some(Value::String(id))
                if bounded(id, 96) && ids.as_ref().is_none_or(|ids| ids.contains(id.as_str())) =>
            {
                Some(id.clone())
            }
            _ => return Err(InspectionError::InvalidAnswer),
        };
        features.push(VisibleFeature {
            name: name.to_owned(),
            look: look.to_owned(),
            region,
            recorded_id,
        });
    }
    Ok(features)
}

fn parse_scene_photo_answer(
    answer: &serde_json::Value,
    spec: &ImageInspection,
) -> Result<InspectionVerdict, InspectionError> {
    let seen = answer
        .get("hero_count")
        .and_then(serde_json::Value::as_u64)
        .ok_or(InspectionError::InvalidAnswer)?;
    // A visibly wrong count remains a rejection even if the new custody label is malformed.
    if seen
        != u64::from(
            spec.expected_heroes
                .ok_or(InspectionError::InvalidRequest)?,
        )
    {
        return Ok(InspectionVerdict::Failed(PhotoCheckCause::HeroCount {
            expected: spec
                .expected_heroes
                .ok_or(InspectionError::InvalidRequest)?,
            seen,
        }));
    }
    if answer.as_object().is_none_or(|fields| fields.len() != 8) {
        return Err(InspectionError::InvalidAnswer);
    }
    let label = |field| {
        answer
            .get(field)
            .and_then(serde_json::Value::as_str)
            .ok_or(InspectionError::InvalidAnswer)
    };
    let frame = match label("frame")? {
        "single_photograph" => PhotoFrame::SinglePhotograph,
        "grid_or_collage" => PhotoFrame::GridOrCollage,
        "cutout_composite" => PhotoFrame::CutoutComposite,
        _ => return Err(InspectionError::InvalidAnswer),
    };
    let medium = match label("medium")? {
        "photograph" => PhotoMedium::Photograph,
        "painting_or_illustration" => PhotoMedium::PaintingOrIllustration,
        "cgi_or_render" => PhotoMedium::CgiOrRender,
        _ => return Err(InspectionError::InvalidAnswer),
    };
    let text = match label("text")? {
        "none" => PhotoText::None,
        "visible_text" => PhotoText::VisibleText,
        _ => return Err(InspectionError::InvalidAnswer),
    };
    let dress = match label("dress")? {
        "period" => PhotoDress::Period,
        "modern_clothing" => PhotoDress::ModernClothing,
        "no_people" => PhotoDress::NoPeople,
        _ => return Err(InspectionError::InvalidAnswer),
    };
    let setting = match label("setting")? {
        "period" => PhotoSetting::Period,
        "modern" => PhotoSetting::Modern,
        _ => return Err(InspectionError::InvalidAnswer),
    };
    let unexpected_drawn_weapon = match label("weapon_custody")? {
        "matches_custody" => false,
        "unexpected_drawn_weapon" => true,
        _ => return Err(InspectionError::InvalidAnswer),
    };
    if unexpected_drawn_weapon {
        return Ok(InspectionVerdict::Failed(
            PhotoCheckCause::UnexpectedDrawnWeapon,
        ));
    }
    Ok(classify(PhotoCheckJudgement {
        kind: InspectionKind::ScenePlate,
        frame,
        medium,
        text,
        dress,
        setting,
        people_count: None,
        people_present: None,
    }))
}

struct PhotoCheckJudgement {
    kind: InspectionKind,
    frame: PhotoFrame,
    medium: PhotoMedium,
    text: PhotoText,
    dress: PhotoDress,
    setting: PhotoSetting,
    people_count: Option<PhotoPeopleCount>,
    people_present: Option<bool>,
}

fn classify(judgement: PhotoCheckJudgement) -> InspectionVerdict {
    let PhotoCheckJudgement {
        kind,
        frame,
        medium,
        text,
        dress,
        setting,
        people_count,
        people_present,
    } = judgement;
    let failure = match frame {
        PhotoFrame::GridOrCollage => Some(PhotoCheckCause::GridOrCollage),
        PhotoFrame::CutoutComposite => Some(PhotoCheckCause::CutoutComposite),
        PhotoFrame::SinglePhotograph => None,
    }
    .or(match medium {
        PhotoMedium::Photograph => None,
        PhotoMedium::PaintingOrIllustration | PhotoMedium::CgiOrRender => {
            Some(PhotoCheckCause::NotPhotograph)
        }
    })
    .or(match setting {
        PhotoSetting::Period => None,
        PhotoSetting::Modern => Some(PhotoCheckCause::ModernSetting),
    })
    .or(match text {
        PhotoText::None => None,
        PhotoText::VisibleText => Some(PhotoCheckCause::VisibleText),
    })
    .or(match (kind, people_present) {
        (InspectionKind::CanonicalRoom, Some(false)) => None,
        (InspectionKind::CanonicalRoom, Some(true)) => Some(PhotoCheckCause::PeopleInEmptyRoom),
        (InspectionKind::CanonicalRoom, None) => Some(PhotoCheckCause::PeopleInEmptyRoom),
        _ => None,
    })
    .or(match dress {
        PhotoDress::ModernClothing => Some(PhotoCheckCause::ModernClothing),
        PhotoDress::Period | PhotoDress::NoPeople => None,
    })
    .or(match (kind, dress, people_count) {
        (InspectionKind::CanonicalCharacter, PhotoDress::NoPeople, _) => {
            Some(PhotoCheckCause::CharacterHasNoPeople)
        }
        (InspectionKind::CanonicalCharacter, _, Some(PhotoPeopleCount::ExactlyOne)) => None,
        (InspectionKind::CanonicalCharacter, _, Some(PhotoPeopleCount::NotOne)) => {
            Some(PhotoCheckCause::CharacterCountNotOne)
        }
        (InspectionKind::CanonicalCharacter, _, None) => {
            Some(PhotoCheckCause::CharacterCountNotOne)
        }
        (InspectionKind::CanonicalRoom, _, _) => None,
        (InspectionKind::ScenePlate, _, _) => None,
    });
    failure.map_or(InspectionVerdict::Pass, InspectionVerdict::Failed)
}

impl InspectionAssessment {
    /// Explicit legacy fail-open evidence never masquerades as a passed judgement.
    pub(crate) fn unjudged(spec: &ImageInspection, reason: UnjudgedReason) -> Self {
        Self {
            contract_revision: spec.contract_revision,
            verdict: InspectionVerdict::Unjudged(reason),
            visible_features: Vec::new(),
            source_binding: None,
        }
    }

    /// Canonical private stage-decision payload; it contains no provider prose or locator.
    pub(crate) fn to_value(&self) -> Value {
        let (verdict, failure, unjudged) = match self.verdict {
            InspectionVerdict::Pass => ("pass", Value::Null, Value::Null),
            InspectionVerdict::Failed(failure) => {
                let (expected, seen) = match failure {
                    PhotoCheckCause::HeroCount { expected, seen } => (json!(expected), json!(seen)),
                    _ => (Value::Null, Value::Null),
                };
                (
                    "failed",
                    json!({
                        "reason":failure.key(),
                        "expected":expected,
                        "seen":seen
                    }),
                    Value::Null,
                )
            }
            InspectionVerdict::Unjudged(reason) => ("unjudged", Value::Null, json!(reason.key())),
        };
        let visible_features = self
            .visible_features
            .iter()
            .map(|feature| {
                json!({
                    "name":feature.name,
                    "look":feature.look,
                    "region":feature.region,
                    "recorded_id":feature.recorded_id,

                })
            })
            .collect::<Vec<_>>();
        let source_binding = self.source_binding.as_ref().map(|binding| {
            json!({
                "place":binding.place,
                "source_hash":binding.source_hash
            })
        });
        json!({
            "schema_revision":1,
            "contract_revision":self.contract_revision,
            "verdict":verdict,
            "failure":failure,
            "unjudged_reason":unjudged,
            "visible_features":visible_features,
            "source_binding":source_binding
        })
    }

    /// Revalidates a durable decision against its frozen inspection contract.
    pub(crate) fn from_value(
        value: &Value,
        spec: &ImageInspection,
    ) -> Result<Self, InspectionError> {
        validate_spec(spec)?;
        let fields = value
            .as_object()
            .filter(|fields| {
                fields.len() == 7
                    && [
                        "schema_revision",
                        "contract_revision",
                        "verdict",
                        "failure",
                        "unjudged_reason",
                        "visible_features",
                        "source_binding",
                    ]
                    .iter()
                    .all(|name| fields.contains_key(*name))
            })
            .ok_or(InspectionError::InvalidAnswer)?;
        // The seven required keys were checked above before indexing this closed object.
        if fields["schema_revision"].as_u64() != Some(1)
            || fields["contract_revision"].as_u64() != Some(u64::from(spec.contract_revision))
            || spec.contract_revision == 0
        {
            return Err(InspectionError::InvalidAnswer);
        }
        let verdict = match fields["verdict"].as_str() {
            Some("pass") if fields["failure"].is_null() && fields["unjudged_reason"].is_null() => {
                InspectionVerdict::Pass
            }
            Some("failed") if fields["unjudged_reason"].is_null() => {
                InspectionVerdict::Failed(parse_failure(&fields["failure"], spec)?)
            }
            Some("unjudged") if fields["failure"].is_null() => {
                InspectionVerdict::Unjudged(match fields["unjudged_reason"].as_str() {
                    Some("preparation") => UnjudgedReason::Preparation,
                    Some("transport") => UnjudgedReason::Transport,
                    Some("provider") => UnjudgedReason::Provider,
                    Some("invalid_response") => UnjudgedReason::InvalidResponse,
                    Some("deadline") => UnjudgedReason::Deadline,
                    Some("disabled") => UnjudgedReason::Disabled,
                    Some("cancelled") => UnjudgedReason::Cancelled,
                    _ => return Err(InspectionError::InvalidAnswer),
                })
            }
            _ => return Err(InspectionError::InvalidAnswer),
        };
        let visible_features = parse_features(&fields["visible_features"], spec)?;
        if !visible_features.is_empty()
            && (verdict != InspectionVerdict::Pass
                || spec.kind != InspectionKind::ScenePlate
                || spec.expected_heroes.is_none())
        {
            return Err(InspectionError::InvalidAnswer);
        }
        let source_binding = match &fields["source_binding"] {
            Value::Null => None,
            Value::Object(binding) if binding.len() == 2 => Some(FeatureBinding {
                place: binding
                    .get("place")
                    .and_then(Value::as_str)
                    .ok_or(InspectionError::InvalidAnswer)?
                    .to_owned(),
                source_hash: binding
                    .get("source_hash")
                    .and_then(Value::as_str)
                    .ok_or(InspectionError::InvalidAnswer)?
                    .to_owned(),
            }),
            _ => return Err(InspectionError::InvalidAnswer),
        };
        if source_binding != binding(spec, !visible_features.is_empty()) {
            return Err(InspectionError::InvalidAnswer);
        }
        Ok(Self {
            contract_revision: spec.contract_revision,
            verdict,
            visible_features,
            source_binding,
        })
    }
}

fn parse_failure(
    value: &Value,
    spec: &ImageInspection,
) -> Result<PhotoCheckCause, InspectionError> {
    let fields = value
        .as_object()
        .filter(|fields| {
            fields.len() == 3
                && ["reason", "expected", "seen"]
                    .iter()
                    .all(|name| fields.contains_key(*name))
        })
        .ok_or(InspectionError::InvalidAnswer)?;
    // All three required failure keys were checked above.
    let failure = match fields["reason"].as_str() {
        Some("grid_or_collage") => PhotoCheckCause::GridOrCollage,
        Some("cutout_composite") => PhotoCheckCause::CutoutComposite,
        Some("not_photograph") => PhotoCheckCause::NotPhotograph,
        Some("modern_setting") => PhotoCheckCause::ModernSetting,
        Some("visible_text") => PhotoCheckCause::VisibleText,
        Some("modern_clothing") => PhotoCheckCause::ModernClothing,
        Some("people_in_empty_room") if spec.kind == InspectionKind::CanonicalRoom => {
            PhotoCheckCause::PeopleInEmptyRoom
        }
        Some("character_count_not_one") if spec.kind == InspectionKind::CanonicalCharacter => {
            PhotoCheckCause::CharacterCountNotOne
        }
        Some("character_has_no_people") if spec.kind == InspectionKind::CanonicalCharacter => {
            PhotoCheckCause::CharacterHasNoPeople
        }
        Some("unexpected_drawn_weapon")
            if spec.kind == InspectionKind::ScenePlate && spec.expected_heroes.is_some() =>
        {
            PhotoCheckCause::UnexpectedDrawnWeapon
        }
        Some("hero_count") if spec.kind == InspectionKind::ScenePlate => {
            let expected = fields["expected"]
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or(InspectionError::InvalidAnswer)?;
            let seen = fields["seen"]
                .as_u64()
                .ok_or(InspectionError::InvalidAnswer)?;
            if Some(expected) != spec.expected_heroes || u64::from(expected) == seen {
                return Err(InspectionError::InvalidAnswer);
            }
            return Ok(PhotoCheckCause::HeroCount { expected, seen });
        }
        _ => return Err(InspectionError::InvalidAnswer),
    };
    if !fields["expected"].is_null() || !fields["seen"].is_null() {
        return Err(InspectionError::InvalidAnswer);
    }
    Ok(failure)
}

#[cfg(test)]
#[path = "../backends/analysis/inspection_policy_tests.rs"]
mod tests;
