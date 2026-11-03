//! Locate: finding each person and item of a plate, and what the engine does when it cannot
//! (D15, J.1, O.16 S2 result).
//!
//! It owns the policy and none of the calls. A segmentation request takes one prompt and returns
//! instances, not identities (J.1), so Locate asks in rounds. Round one asks for the people once
//! and for each item's first phrase. An item that comes back with no instance is asked again with
//! its next phrase, up to three (S2: 73 percent of items were found on the first phrase and 87
//! percent after one retry with a plain head noun). Each person takes the instance nearest the
//! position the rules owner's layout states, each instance goes to one person at most, and an item
//! takes the instance nearest its stated position.
//!
//! Nothing is dropped silently. A person with no instance is `Unlocated` with an explicit
//! fallback point; an item that no phrase found falls back to its own picture at its stated
//! position, or is `Unlocated` too when it has none; a call that failed ends that target's
//! attempts and says so in its reason (a failure is not a miss, and asking again would only spend
//! again). Every target the plan was made with appears exactly once in the outcome. In combat the
//! rules owner's state is the truth for reach and cover and this outcome only places overlays and
//! hotspots (RES1).
//!
//! It makes no request, reads no clock and never sees the picture: the host turns each prompt
//! into one broker request and hands the answer back.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::element::ElementId;
use crate::event::FailureCause;
use crate::scene_plan_brief::{LocatePhrase, MAXIMUM_PHRASES_PER_ITEM};
use crate::table_state::{MediaName, Placement, PlacementError};

/// The canvas width and the foreground lane in thousandths of a canvas unit: where a target with
/// no stated position is put until it is found. The lane is 70 percent of the way down, in front
/// of the set.
const CANVAS_WIDTH_MILLI_UNITS: u32 = 100_000;
const FOREGROUND_LANE_MILLI_UNITS: u32 = 39_375;
/// The highest score a segmentation instance has, in thousandths.
const MAXIMUM_SCORE_PERMILLE: u16 = 1_000;

/// Why a region or an instance was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionError {
    /// A region has positive width and height.
    Empty,
    OffCanvas,
    ScoreAboveOne,
}

impl fmt::Display for RegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("a region has width and height"),
            Self::OffCanvas => formatter.write_str("a region is on the canvas"),
            Self::ScoreAboveOne => formatter.write_str("a score is at most 1"),
        }
    }
}

impl std::error::Error for RegionError {}

impl From<PlacementError> for RegionError {
    fn from(_: PlacementError) -> Self {
        Self::OffCanvas
    }
}

/// A box on the 100 by 56.25 canvas, with the two points the screen needs: its centre (for a
/// hotspot) and its head point, the middle of its top edge (for a speech mark).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    x_min_milli_units: u32,
    y_min_milli_units: u32,
    x_max_milli_units: u32,
    y_max_milli_units: u32,
    centre: Placement,
    head: Placement,
}

impl Region {
    pub fn new(
        x_min_milli_units: u32,
        y_min_milli_units: u32,
        x_max_milli_units: u32,
        y_max_milli_units: u32,
    ) -> Result<Self, RegionError> {
        if x_min_milli_units >= x_max_milli_units || y_min_milli_units >= y_max_milli_units {
            return Err(RegionError::Empty);
        }
        Placement::from_milli_units(x_min_milli_units, y_min_milli_units)?;
        Placement::from_milli_units(x_max_milli_units, y_max_milli_units)?;
        let middle_x = (x_min_milli_units + x_max_milli_units) / 2;
        let middle_y = (y_min_milli_units + y_max_milli_units) / 2;
        Ok(Self {
            x_min_milli_units,
            y_min_milli_units,
            x_max_milli_units,
            y_max_milli_units,
            centre: Placement::from_milli_units(middle_x, middle_y)?,
            head: Placement::from_milli_units(middle_x, y_min_milli_units)?,
        })
    }

    /// The corners, as thousandths of a canvas unit: left, top, right, bottom.
    pub fn bounds_milli_units(&self) -> (u32, u32, u32, u32) {
        (
            self.x_min_milli_units,
            self.y_min_milli_units,
            self.x_max_milli_units,
            self.y_max_milli_units,
        )
    }

    pub fn centre(&self) -> Placement {
        self.centre
    }

    /// The middle of the top edge: where a figure's head is, until a head prompt says better.
    pub fn head(&self) -> Placement {
        self.head
    }
}

/// One thing the segmentation model found: a region and how sure it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    region: Region,
    score_permille: u16,
}

impl Instance {
    pub fn new(region: Region, score_permille: u16) -> Result<Self, RegionError> {
        if score_permille > MAXIMUM_SCORE_PERMILLE {
            return Err(RegionError::ScoreAboveOne);
        }
        Ok(Self {
            region,
            score_permille,
        })
    }

    pub fn region(&self) -> Region {
        self.region
    }

    pub fn score_permille(&self) -> u16 {
        self.score_permille
    }
}

/// A person Locate looks for, with where the layout says they stand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FigureTarget {
    pub id: ElementId,
    pub stated: Option<Placement>,
}

/// An item Locate looks for: its phrases, best first, its stated position and its own picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemTarget {
    pub id: ElementId,
    pub stated: Option<Placement>,
    pub phrases: Vec<LocatePhrase>,
    pub picture: Option<MediaName>,
}

/// What one segmentation request asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocatePrompt {
    /// Every person: one request, whatever the cast size (J.1).
    People,
    /// One item, by one phrase; `attempt` counts from 1.
    Item {
        item: ElementId,
        phrase: LocatePhrase,
        attempt: u8,
    },
}

/// Why a target has no place in the plate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnlocatedReason {
    /// The people request found fewer instances than there are people.
    NoInstance,
    /// Every phrase came back with nothing and the item has no picture of its own.
    NoPhraseMatched,
    /// The request failed, so asking again was not tried.
    LocateFailed { cause: FailureCause },
    /// The plan ended before the target was asked about.
    NotAsked,
}

impl UnlocatedReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::NoInstance => "no_instance",
            Self::NoPhraseMatched => "no_phrase_matched",
            Self::LocateFailed { .. } => "locate_failed",
            Self::NotAsked => "not_asked",
        }
    }
}

/// Where a target is, or the explicit point the screen uses instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "anchor", rename_all = "snake_case")]
pub enum Anchor {
    Located {
        region: Region,
        head: Placement,
    },
    /// An item no phrase found: its own picture, at its stated position.
    OwnPicture {
        at: Placement,
        picture: MediaName,
    },
    Unlocated {
        fallback: Placement,
        reason: UnlocatedReason,
    },
}

impl Anchor {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Located { .. } => "located",
            Self::OwnPicture { .. } => "own_picture",
            Self::Unlocated { .. } => "unlocated",
        }
    }
}

/// A person's anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocatedFigure {
    pub id: ElementId,
    pub anchor: Anchor,
}

/// An item's anchor and how many phrases it took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocatedItem {
    pub id: ElementId,
    pub anchor: Anchor,
    /// The phrase, counted from 1, that found it; `None` when none did.
    pub found_by_phrase: Option<u8>,
    pub phrases_tried: u8,
}

/// Everything Locate decided about one plate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocateOutcome {
    pub figures: Vec<LocatedFigure>,
    pub items: Vec<LocatedItem>,
}

impl LocateOutcome {
    /// An outcome that found nothing, for a Locate that could not run: every target is
    /// `Unlocated` at its fallback point for `reason`, never absent.
    pub fn unlocated(
        figures: &[FigureTarget],
        items: &[ItemTarget],
        reason: UnlocatedReason,
    ) -> Result<Self, LocateError> {
        let mut plan = LocatePlan::new(figures.to_vec(), items.to_vec())?;
        plan.give_up(reason);
        Ok(plan.finish())
    }

    pub fn located_figures(&self) -> usize {
        self.figures
            .iter()
            .filter(|figure| matches!(figure.anchor, Anchor::Located { .. }))
            .count()
    }

    pub fn unlocated_figures(&self) -> usize {
        self.figures.len() - self.located_figures()
    }

    pub fn located_items(&self) -> usize {
        self.count_items("located")
    }

    pub fn own_picture_items(&self) -> usize {
        self.count_items("own_picture")
    }

    pub fn unlocated_items(&self) -> usize {
        self.count_items("unlocated")
    }

    fn count_items(&self, key: &str) -> usize {
        self.items
            .iter()
            .filter(|item| item.anchor.key() == key)
            .count()
    }
}

/// Why the plan refused something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocateError {
    /// The answer is for a prompt that is not waiting for one.
    NotAsked,
    /// An item has more phrases than Locate tries.
    TooManyPhrases { item: ElementId },
    /// An item with no phrase cannot be looked for.
    NoPhrase { item: ElementId },
    /// A fallback point fell off the canvas, which a stated position cannot.
    OffCanvas,
}

impl fmt::Display for LocateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAsked => formatter.write_str("that prompt is not waiting for an answer"),
            Self::TooManyPhrases { item } => write!(
                formatter,
                "{item} has more than {MAXIMUM_PHRASES_PER_ITEM} phrases"
            ),
            Self::NoPhrase { item } => write!(formatter, "{item} has no phrase to look for"),
            Self::OffCanvas => formatter.write_str("a fallback point is on the canvas"),
        }
    }
}

impl std::error::Error for LocateError {}

impl From<PlacementError> for LocateError {
    fn from(_: PlacementError) -> Self {
        Self::OffCanvas
    }
}

/// The point a target stands at when it is not found: its stated position, else its slot along
/// the foreground lane, spread evenly so two lost targets are not on top of each other.
fn fallback_point(
    stated: Option<Placement>,
    slot: usize,
    of: usize,
) -> Result<Placement, PlacementError> {
    if let Some(stated) = stated {
        return Ok(stated);
    }
    let divisions = u64::try_from(of).unwrap_or(u64::MAX).saturating_add(1);
    let step = u64::try_from(slot).unwrap_or(u64::MAX).saturating_add(1);
    let x = u64::from(CANVAS_WIDTH_MILLI_UNITS).saturating_mul(step) / divisions;
    let x = u32::try_from(x).unwrap_or(CANVAS_WIDTH_MILLI_UNITS);
    Placement::from_milli_units(x, FOREGROUND_LANE_MILLI_UNITS)
}

fn squared_distance(from: Placement, to: Placement) -> f64 {
    let dx = f64::from(from.x_units()) - f64::from(to.x_units());
    let dy = f64::from(from.y_units()) - f64::from(to.y_units());
    dx * dx + dy * dy
}

/// The instance nearest `stated`, the most sure on a tie; with no stated position, the most sure.
fn nearest_instance(stated: Option<Placement>, instances: &[Instance]) -> Option<usize> {
    let key = |instance: &Instance| {
        let distance = stated.map_or(0.0, |at| squared_distance(at, instance.region.centre()));
        (distance, std::cmp::Reverse(instance.score_permille))
    };
    (0..instances.len()).min_by(|left, right| {
        let (left_distance, left_score) = key(&instances[*left]);
        let (right_distance, right_score) = key(&instances[*right]);
        left_distance
            .total_cmp(&right_distance)
            .then(left_score.cmp(&right_score))
            .then(left.cmp(right))
    })
}

/// The instance each person takes, by index into `figures`, each instance at most once. People
/// with a stated position are matched first, the closest pair first; the others take the most
/// sure instances left, in the order the people are listed.
fn assign_people(figures: &[FigureTarget], instances: &[Instance]) -> Vec<Option<usize>> {
    let mut taken = vec![false; instances.len()];
    let mut assigned = vec![None; figures.len()];
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (figure_index, figure) in figures.iter().enumerate() {
        let Some(stated) = figure.stated else {
            continue;
        };
        for (instance_index, instance) in instances.iter().enumerate() {
            pairs.push((
                squared_distance(stated, instance.region.centre()),
                figure_index,
                instance_index,
            ));
        }
    }
    pairs.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then(left.1.cmp(&right.1))
            .then(left.2.cmp(&right.2))
    });
    for (_, figure_index, instance_index) in pairs {
        if assigned[figure_index].is_none() && !taken[instance_index] {
            assigned[figure_index] = Some(instance_index);
            taken[instance_index] = true;
        }
    }
    let mut remaining: Vec<usize> = (0..instances.len())
        .filter(|index| !taken[*index])
        .collect();
    remaining.sort_by(|left, right| {
        instances[*right]
            .score_permille
            .cmp(&instances[*left].score_permille)
            .then(left.cmp(right))
    });
    let mut remaining = remaining.into_iter();
    for (figure_index, figure) in figures.iter().enumerate() {
        if figure.stated.is_none()
            && let Some(instance_index) = remaining.next()
        {
            assigned[figure_index] = Some(instance_index);
        }
    }
    assigned
}

#[derive(Debug)]
enum PeopleProgress {
    NotAsked,
    Asked,
    Answered(Vec<Instance>),
    Failed(FailureCause),
    GaveUp(UnlocatedReason),
}

#[derive(Debug)]
enum ItemResult {
    Found { instance: Instance, phrase: u8 },
    Failed(FailureCause),
}

#[derive(Debug)]
struct ItemProgress {
    target: ItemTarget,
    fallback: Placement,
    next_phrase: usize,
    tried: u8,
    asked: bool,
    result: Option<ItemResult>,
    gave_up: Option<UnlocatedReason>,
}

/// One plate's Locate, as the rounds of prompts and the outcome they add up to.
#[derive(Debug)]
pub struct LocatePlan {
    figures: Vec<FigureTarget>,
    figure_fallbacks: Vec<Placement>,
    people: PeopleProgress,
    items: Vec<ItemProgress>,
}

impl LocatePlan {
    /// A plan for these targets. An item needs one to three phrases, because an item with none is
    /// not looked for and does not belong here.
    pub fn new(figures: Vec<FigureTarget>, items: Vec<ItemTarget>) -> Result<Self, LocateError> {
        let figure_count = figures.len();
        let figure_fallbacks = figures
            .iter()
            .enumerate()
            .map(|(slot, figure)| fallback_point(figure.stated, slot, figure_count))
            .collect::<Result<Vec<_>, _>>()?;
        let item_count = items.len();
        let mut progress = Vec::with_capacity(item_count);
        for (slot, target) in items.into_iter().enumerate() {
            if target.phrases.is_empty() {
                return Err(LocateError::NoPhrase { item: target.id });
            }
            if target.phrases.len() > MAXIMUM_PHRASES_PER_ITEM {
                return Err(LocateError::TooManyPhrases { item: target.id });
            }
            progress.push(ItemProgress {
                fallback: fallback_point(target.stated, slot, item_count)?,
                target,
                next_phrase: 0,
                tried: 0,
                asked: false,
                result: None,
                gave_up: None,
            });
        }
        Ok(Self {
            people: if figures.is_empty() {
                PeopleProgress::Answered(Vec::new())
            } else {
                PeopleProgress::NotAsked
            },
            figures,
            figure_fallbacks,
            items: progress,
        })
    }

    /// The prompts to send now, each as one request. It is empty when every answer is in, or
    /// when what is left waits on answers already asked for.
    pub fn next_round(&mut self) -> Vec<LocatePrompt> {
        let mut prompts = Vec::new();
        if matches!(self.people, PeopleProgress::NotAsked) {
            self.people = PeopleProgress::Asked;
            prompts.push(LocatePrompt::People);
        }
        for item in &mut self.items {
            if item.asked || item.result.is_some() || item.gave_up.is_some() {
                continue;
            }
            let Some(phrase) = item.target.phrases.get(item.next_phrase) else {
                continue;
            };
            item.asked = true;
            prompts.push(LocatePrompt::Item {
                item: item.target.id.clone(),
                phrase: phrase.clone(),
                attempt: attempt_number(item.next_phrase),
            });
        }
        prompts
    }

    /// Whether any prompt is still waiting for an answer.
    pub fn is_waiting(&self) -> bool {
        matches!(self.people, PeopleProgress::Asked) || self.items.iter().any(|item| item.asked)
    }

    /// Whether every target has its answer, so `finish` adds nothing to guess at.
    pub fn is_settled(&self) -> bool {
        let people_settled = !matches!(
            self.people,
            PeopleProgress::NotAsked | PeopleProgress::Asked
        );
        people_settled
            && self.items.iter().all(|item| {
                item.result.is_some()
                    || item.gave_up.is_some()
                    || item.next_phrase >= item.target.phrases.len()
            })
    }

    /// Takes the answer to one prompt: the instances found, possibly none, or why the call failed.
    pub fn record(
        &mut self,
        prompt: &LocatePrompt,
        answer: Result<Vec<Instance>, FailureCause>,
    ) -> Result<(), LocateError> {
        match prompt {
            LocatePrompt::People => {
                if !matches!(self.people, PeopleProgress::Asked) {
                    return Err(LocateError::NotAsked);
                }
                self.people = match answer {
                    Ok(instances) => PeopleProgress::Answered(instances),
                    Err(cause) => PeopleProgress::Failed(cause),
                };
            }
            LocatePrompt::Item { item, attempt, .. } => {
                let progress = self
                    .items
                    .iter_mut()
                    .find(|progress| {
                        progress.target.id == *item
                            && progress.asked
                            && attempt_number(progress.next_phrase) == *attempt
                    })
                    .ok_or(LocateError::NotAsked)?;
                progress.asked = false;
                progress.tried = progress.tried.saturating_add(1);
                match answer {
                    Ok(instances) => match nearest_instance(progress.target.stated, &instances) {
                        Some(index) => {
                            progress.result = Some(ItemResult::Found {
                                instance: instances[index],
                                phrase: *attempt,
                            });
                        }
                        None => progress.next_phrase += 1,
                    },
                    Err(cause) => progress.result = Some(ItemResult::Failed(cause)),
                }
            }
        }
        Ok(())
    }

    /// Stops asking: every target not answered yet is `Unlocated` for `reason`.
    pub fn give_up(&mut self, reason: UnlocatedReason) {
        if matches!(
            self.people,
            PeopleProgress::NotAsked | PeopleProgress::Asked
        ) {
            self.people = PeopleProgress::GaveUp(reason);
        }
        for item in &mut self.items {
            if item.result.is_none() {
                item.gave_up = Some(reason);
                item.asked = false;
            }
        }
    }

    /// The outcome, every target once. A target the plan never got an answer for is `Unlocated`
    /// with the reason `NotAsked`.
    pub fn finish(self) -> LocateOutcome {
        let figure_anchors = self.figure_anchors();
        let figures = self
            .figures
            .into_iter()
            .zip(figure_anchors)
            .map(|(figure, anchor)| LocatedFigure {
                id: figure.id,
                anchor,
            })
            .collect();
        let items = self.items.into_iter().map(item_outcome).collect();
        LocateOutcome { figures, items }
    }

    fn figure_anchors(&self) -> Vec<Anchor> {
        let unlocated = |index: usize, reason| Anchor::Unlocated {
            fallback: self.figure_fallbacks[index],
            reason,
        };
        let reason_for_all = match &self.people {
            PeopleProgress::Failed(cause) => Some(UnlocatedReason::LocateFailed { cause: *cause }),
            PeopleProgress::GaveUp(reason) => Some(*reason),
            PeopleProgress::NotAsked | PeopleProgress::Asked => Some(UnlocatedReason::NotAsked),
            PeopleProgress::Answered(_) => None,
        };
        if let Some(reason) = reason_for_all {
            return (0..self.figures.len())
                .map(|index| unlocated(index, reason))
                .collect();
        }
        let PeopleProgress::Answered(instances) = &self.people else {
            return Vec::new();
        };
        assign_people(&self.figures, instances)
            .into_iter()
            .enumerate()
            .map(|(index, taken)| match taken {
                Some(instance_index) => {
                    let region = instances[instance_index].region;
                    Anchor::Located {
                        region,
                        head: region.head(),
                    }
                }
                None => unlocated(index, UnlocatedReason::NoInstance),
            })
            .collect()
    }
}

fn attempt_number(phrase_index: usize) -> u8 {
    u8::try_from(phrase_index.saturating_add(1)).unwrap_or(u8::MAX)
}

fn item_outcome(item: ItemProgress) -> LocatedItem {
    let ItemProgress {
        target,
        fallback,
        tried,
        result,
        gave_up,
        ..
    } = item;
    let own_picture = |reason| match target.picture.clone() {
        Some(picture) => Anchor::OwnPicture {
            at: fallback,
            picture,
        },
        None => Anchor::Unlocated { fallback, reason },
    };
    let (anchor, found_by_phrase) = match result {
        Some(ItemResult::Found { instance, phrase }) => (
            Anchor::Located {
                region: instance.region,
                head: instance.region.head(),
            },
            Some(phrase),
        ),
        Some(ItemResult::Failed(cause)) => {
            (own_picture(UnlocatedReason::LocateFailed { cause }), None)
        }
        None => {
            let exhausted = usize::from(tried) >= target.phrases.len();
            let reason = if exhausted {
                UnlocatedReason::NoPhraseMatched
            } else {
                gave_up.unwrap_or(UnlocatedReason::NotAsked)
            };
            (own_picture(reason), None)
        }
    };
    LocatedItem {
        id: target.id,
        anchor,
        found_by_phrase,
        phrases_tried: tried,
    }
}
