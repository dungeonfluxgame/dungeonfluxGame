//! The purposes of the scene builder's stages: which `PurposeSpec` each stage asks the broker for,
//! and the ones the broker's built-in table does not have (J.1, J.4, H.7, O.16 SB).
//!
//! Each stage is one kind of broker request. Describe is `describe` (live text), Background is
//! `background` (live picture, with the establishing card as its fallback), Plate is `plate` (the
//! plate class, holding the last plate when it fails) and Locate is `locate`. Locate is one
//! segmentation request per prompt (J.1), priced at the fal list price of SAM 3.1, USD 0.01 a
//! request (`content/pricing.json`, 2026-10-24); J.4 sets its retry at one call retry. It joins the
//! plate class, so a plate and the anchors it needs are served together and a full plate queue drops
//! the oldest first (H.7). Its answer is the instances in a closed JSON schema (`answers`), because
//! a broker result carries text or a picture and no richer value.
//!
//! The same four stages exist at the look-ahead class (SB-LA, "Ideas to come" 1): the builder
//! takes a `BuildPriority` and asks for the look-ahead purposes when it is `Lookahead`, so the
//! look-ahead worker can reuse the builder without a second pipeline. `lookahead_plate` and
//! `lookahead_locate` are added here; `lookahead_describe` and `lookahead_background` are the
//! broker's. A look-ahead build never gates anything and its fallback is `Nothing`.
//!
//! It adds specs and never changes the broker's: `scene_purpose_table` is the built-in table plus
//! these three entries.

use df_table::element::NameError;
use df_table::plate::cost::{CostView, StagePrice, StageUse};
use df_table::plate::timing::PlateTiming;
use df_table::ports::PurposeName;
use df_table::scene_plan::SceneStage;
use df_table::scene_plan::brief::SceneBrief;

use super::request::DevStage;
use crate::broker::limits::MAXIMUM_TEXT_ANSWER_CHARACTERS;
use crate::broker::purpose::{
    Fallback, IMAGE_DEADLINE_MS, Price, PurposeError, PurposeFacts, PurposeSpec, PurposeTable,
    QueueClass, ResourceKind, RetryPolicy, SizeRule,
};

/// How long one segmentation request may take. The S2 median was 1.5 s and the slowest 3 s, so
/// 30 s is about twenty times the median: a candidate, to be tuned from measurement (the plan
/// gives none).
pub const LOCATE_DEADLINE_MS: u64 = 30_000;
/// The fal list price of one SAM 3.1 request, in micro-dollars (USD 0.01, `content/pricing.json`).
pub const LOCATE_PRICE_MICRO_DOLLARS: u64 = 10_000;
/// J.4: SAM retries are one call retry. The backoff is the broker's short one for a text-lane call.
const LOCATE_RETRY: RetryPolicy = RetryPolicy {
    maximum_retries: 1,
    initial_backoff_ms: 500,
    maximum_backoff_ms: 2_000,
};

const DESCRIBE: &str = "describe";
const BACKGROUND: &str = "background";
const PLATE: &str = "plate";
const LOCATE: &str = "locate";
const LOOKAHEAD_DESCRIBE: &str = "lookahead_describe";
const LOOKAHEAD_BACKGROUND: &str = "lookahead_background";
const LOOKAHEAD_PLATE: &str = "lookahead_plate";
const LOOKAHEAD_LOCATE: &str = "lookahead_locate";

/// Which queue class a build waits in: the table's own scene, or the next one built early.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuildPriority {
    Current,
    /// SB-LA: the lowest class below the current scene's own work.
    Lookahead,
}

impl BuildPriority {
    pub fn key(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Lookahead => "lookahead",
        }
    }
}

/// The purpose of each stage that asks the broker for something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagePurposes {
    pub describe: PurposeName,
    pub background: PurposeName,
    pub plate: PurposeName,
    pub locate: PurposeName,
}

impl StagePurposes {
    pub fn for_priority(priority: BuildPriority) -> Result<Self, NameError> {
        let (describe, background, plate, locate) = match priority {
            BuildPriority::Current => (DESCRIBE, BACKGROUND, PLATE, LOCATE),
            BuildPriority::Lookahead => (
                LOOKAHEAD_DESCRIBE,
                LOOKAHEAD_BACKGROUND,
                LOOKAHEAD_PLATE,
                LOOKAHEAD_LOCATE,
            ),
        };
        Ok(Self {
            describe: PurposeName::new(describe)?,
            background: PurposeName::new(background)?,
            plate: PurposeName::new(plate)?,
            locate: PurposeName::new(locate)?,
        })
    }

    /// The purpose of a stage, or `None` for Ready, which asks for nothing.
    pub fn of(&self, stage: SceneStage) -> Option<&PurposeName> {
        match stage {
            SceneStage::Describe => Some(&self.describe),
            SceneStage::Background => Some(&self.background),
            SceneStage::Plate => Some(&self.plate),
            SceneStage::Locate => Some(&self.locate),
            SceneStage::Ready => None,
        }
    }

    /// The purpose of a stage a dev call slows down or makes fail; each of them asks the broker.
    pub fn of_dev_stage(&self, stage: DevStage) -> &PurposeName {
        match stage {
            DevStage::Describe => &self.describe,
            DevStage::Background => &self.background,
            DevStage::Plate => &self.plate,
            DevStage::Locate => &self.locate,
        }
    }
}

/// The purpose a dev call scripts for `stage` of a build at `priority`.
pub fn dev_stage_purpose(
    priority: BuildPriority,
    stage: DevStage,
) -> Result<PurposeName, NameError> {
    Ok(StagePurposes::for_priority(priority)?
        .of_dev_stage(stage)
        .clone())
}

fn locate_spec(name: &str, class: QueueClass) -> Result<PurposeSpec, PurposeError> {
    PurposeSpec::new(
        name,
        PurposeFacts {
            kind: ResourceKind::Text,
            deadline_ms: LOCATE_DEADLINE_MS,
            size: SizeRule::TextCharacters {
                maximum: MAXIMUM_TEXT_ANSWER_CHARACTERS,
            },
            retry: LOCATE_RETRY,
            class,
            price: Price::Priced {
                micro_dollars: LOCATE_PRICE_MICRO_DOLLARS,
            },
            // The broker's closed list has no "mark every figure unlocated" fallback; Locate's
            // failure is the typed `LocateOutcome` with a fallback point for each target.
            fallback: Fallback::Nothing,
        },
    )
}

fn lookahead_plate_spec() -> Result<PurposeSpec, PurposeError> {
    let width = crate::broker::limits::MAXIMUM_IMAGE_ANSWER_EDGE_PIXELS;
    PurposeSpec::new(
        LOOKAHEAD_PLATE,
        PurposeFacts {
            kind: ResourceKind::Image,
            deadline_ms: IMAGE_DEADLINE_MS,
            size: SizeRule::ImagePixels {
                width,
                height: width,
            },
            retry: RetryPolicy {
                maximum_retries: 2,
                initial_backoff_ms: 500,
                maximum_backoff_ms: 8_000,
            },
            class: QueueClass::Lookahead,
            price: Price::Unpriced,
            fallback: Fallback::Nothing,
        },
    )
}

/// The three purposes the scene builder adds to the broker's.
pub fn scene_purposes() -> Result<Vec<PurposeSpec>, PurposeError> {
    Ok(vec![
        locate_spec(LOCATE, QueueClass::Plate)?,
        locate_spec(LOOKAHEAD_LOCATE, QueueClass::Lookahead)?,
        lookahead_plate_spec()?,
    ])
}

/// The broker's built-in purposes and the scene builder's.
pub fn scene_purpose_table() -> Result<PurposeTable, PurposeError> {
    let built_in = PurposeTable::built_in()?;
    let mut specs: Vec<PurposeSpec> = built_in.all().cloned().collect();
    specs.extend(scene_purposes()?);
    PurposeTable::new(specs)
}

fn stage_price(price: Price) -> StagePrice {
    match price {
        Price::Priced { micro_dollars } => StagePrice::Priced { micro_dollars },
        Price::Unpriced => StagePrice::Unpriced,
    }
}

/// The calls one plate of `brief`'s scene makes, from the purposes' prices: one plate request,
/// and Locate's one request for the people and between one and three for each item (S2).
pub fn stage_uses(
    table: &PurposeTable,
    purposes: &StagePurposes,
    brief: &SceneBrief,
    has_people: bool,
) -> Vec<StageUse> {
    let items = brief.set().iter().filter(|piece| piece.is_item());
    let least = u32::from(has_people) + u32::try_from(items.clone().count()).unwrap_or(u32::MAX);
    let most = u32::from(has_people)
        + items
            .map(|piece| u32::try_from(piece.phrases().len()).unwrap_or(u32::MAX))
            .sum::<u32>();
    [(&purposes.plate, 1, 1), (&purposes.locate, least, most)]
        .into_iter()
        .filter_map(|(purpose, calls_least, calls_most)| {
            let spec = table.get(purpose)?;
            Some(StageUse {
                purpose: purpose.clone(),
                price: stage_price(spec.price()),
                calls_least_per_plate: calls_least,
                calls_most_per_plate: calls_most,
            })
        })
        .collect()
}

/// Plates per hour and the cost per hour of `brief`'s scene at the plate interval (D18).
pub fn cost_view(
    timing: PlateTiming,
    table: &PurposeTable,
    purposes: &StagePurposes,
    brief: &SceneBrief,
    has_people: bool,
) -> CostView {
    CostView::estimate(timing, &stage_uses(table, purposes, brief, has_people))
}
