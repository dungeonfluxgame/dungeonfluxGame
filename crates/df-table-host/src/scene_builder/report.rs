//! The read-only views of the scene builder: what each open scene has stored, where its jobs
//! stand, what Locate decided and where the establishing card is.
//!
//! These are the data behind `scene show` (H.16, coding style section 8, pattern 8). A report is a
//! copy: building it changes nothing and holds no lock past the call. It carries ids, closed keys,
//! counts and positions, and never a description, a phrase or any player text, except the place
//! name, which is a short display name of the place and not something a player typed.

use df_table::event::FailureCause;
use df_table::plate::job::JobReason;
use df_table::scene_plan::card::CardState;
use df_table::scene_plan::locate::{Anchor, LocateOutcome};
use df_table::scene_plan::{SceneStage, StageOutputs};
use df_table::table_state::{Placement, TableSnapshot};
use serde::{Deserialize, Serialize};

use super::builder::SceneBuilder;

/// A point on the canvas in canvas units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointReport {
    pub x: f32,
    pub y: f32,
}

impl From<Placement> for PointReport {
    fn from(placement: Placement) -> Self {
        Self {
            x: placement.x_units(),
            y: placement.y_units(),
        }
    }
}

/// One person's or item's anchor, with the explicit fallback point when it was not found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnchorReport {
    pub id: String,
    /// `located`, `own_picture` or `unlocated`.
    pub anchor: String,
    /// Where it is: the region's centre when located, the fallback point otherwise.
    pub at: PointReport,
    pub head: Option<PointReport>,
    /// Why it is unlocated.
    pub reason: Option<String>,
    /// For an item: the phrase, counted from 1, that found it.
    pub found_by_phrase: Option<u8>,
    pub phrases_tried: Option<u8>,
}

/// What Locate decided for one plate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocateReport {
    pub figures: Vec<AnchorReport>,
    pub items: Vec<AnchorReport>,
}

fn anchor_report(
    id: &str,
    anchor: &Anchor,
    found_by_phrase: Option<u8>,
    phrases_tried: Option<u8>,
) -> AnchorReport {
    let (at, head, reason) = match anchor {
        Anchor::Located { region, head } => (
            PointReport::from(region.centre()),
            Some(PointReport::from(*head)),
            None,
        ),
        Anchor::OwnPicture { at, .. } => (PointReport::from(*at), None, None),
        Anchor::Unlocated { fallback, reason } => (
            PointReport::from(*fallback),
            None,
            Some(reason.key().to_owned()),
        ),
    };
    AnchorReport {
        id: id.to_owned(),
        anchor: anchor.key().to_owned(),
        at,
        head,
        reason,
        found_by_phrase,
        phrases_tried,
    }
}

impl LocateReport {
    pub fn of(outcome: &LocateOutcome) -> Self {
        Self {
            figures: outcome
                .figures
                .iter()
                .map(|figure| anchor_report(figure.id.as_str(), &figure.anchor, None, None))
                .collect(),
            items: outcome
                .items
                .iter()
                .map(|item| {
                    anchor_report(
                        item.id.as_str(),
                        &item.anchor,
                        item.found_by_phrase,
                        Some(item.phrases_tried),
                    )
                })
                .collect(),
        }
    }
}

/// A stage that failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureReport {
    pub stage: SceneStage,
    pub cause: FailureCause,
}

/// One plate job in flight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobReport {
    pub job: u64,
    pub tick_ms: u64,
    /// Why the job was taken.
    pub reason: JobReason,
    pub figures: usize,
    /// The stage whose request is out.
    pub running: Option<SceneStage>,
    /// The stages with a stored output.
    pub stored: Vec<SceneStage>,
    pub failure: Option<FailureReport>,
    pub locate: Option<LocateReport>,
}

/// The establishing card and its timing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardReport {
    /// `building`, `showing` or `retired`.
    pub state: String,
    pub due_at_ms: u64,
    pub late_by_ms: Option<u64>,
    pub has_narrator_line: bool,
    pub party: Vec<String>,
    pub place_music: Option<String>,
    pub background_stored: bool,
    pub tag: String,
}

/// One open scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneReport {
    pub scene: u64,
    pub generation: u64,
    pub place: String,
    pub kind: String,
    pub priority: String,
    pub started_ms: u64,
    pub description_stored: bool,
    pub background_stored: bool,
    pub first_plate_ready: bool,
    pub card: CardReport,
    pub jobs: Vec<JobReport>,
}

fn stored_stages(outputs: &StageOutputs) -> Vec<SceneStage> {
    SceneStage::ALL
        .iter()
        .copied()
        .filter(|stage| outputs.has(*stage))
        .collect()
}

impl SceneBuilder {
    /// Every open scene, in scene order. `plate_on_screen` says whether a scene's plate is on screen.
    pub fn report(
        &self,
        now_ms: u64,
        snapshot: &TableSnapshot,
        plate_on_screen: impl Fn(df_table::identity::SceneId) -> bool,
    ) -> Vec<SceneReport> {
        self.records()
            .map(|(scene, record)| {
                let on_screen = plate_on_screen(*scene);
                let state = record.watch.state(now_ms, on_screen);
                let card = df_table::scene_plan::card::EstablishingCard::compose(
                    &record.brief,
                    &record.outputs,
                    snapshot,
                );
                SceneReport {
                    scene: scene.get(),
                    generation: record.generation.get(),
                    place: record.brief.place().as_str().to_owned(),
                    kind: record.kind.key().to_owned(),
                    priority: record.priority.key().to_owned(),
                    started_ms: record.started_ms,
                    description_stored: record.outputs.description.is_some(),
                    background_stored: record.outputs.background.is_some(),
                    first_plate_ready: record.first_plate_ready,
                    card: CardReport {
                        state: state.key().to_owned(),
                        due_at_ms: record.watch.due_at_ms(),
                        late_by_ms: match state {
                            CardState::Showing { late_by_ms } => Some(late_by_ms),
                            CardState::Building { .. } | CardState::Retired => None,
                        },
                        has_narrator_line: card.narrator_line.is_some(),
                        party: card
                            .party
                            .iter()
                            .map(|chip| chip.name.as_str().to_owned())
                            .collect(),
                        place_music: card.place_music.map(|music| music.as_str().to_owned()),
                        background_stored: card.background.is_some(),
                        tag: card.tag.key().to_owned(),
                    },
                    jobs: record
                        .jobs
                        .iter()
                        .map(|run| {
                            let plan = run.plan();
                            JobReport {
                                job: plan.job().id().get(),
                                tick_ms: plan.job().tick_ms(),
                                reason: plan.job().reason(),
                                figures: plan.job().figures().len(),
                                running: plan.running(),
                                stored: stored_stages(plan.outputs()),
                                failure: plan.failure().map(|failure| FailureReport {
                                    stage: failure.stage,
                                    cause: failure.cause,
                                }),
                                locate: plan.outputs().locate.as_ref().map(LocateReport::of),
                            }
                        })
                        .collect(),
                }
            })
            .collect()
    }
}
