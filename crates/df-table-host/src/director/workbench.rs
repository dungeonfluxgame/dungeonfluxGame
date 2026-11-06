//! The Director on the host: its controlled clock, the stand-in view of the shared screen and the
//! translation of a verb into the Director's calls (O.14, G1).
//!
//! The workbench is synchronous bookkeeping around `df_table::director::Director`. It reads the
//! time from the clock it is given (the host's monotonic clock, or a manual one in a test) plus
//! whatever `director advance` added, builds the `StageView` the Director decides on from
//! the stage's element names and the operator's view edits, and returns the directions and audit
//! lines of one verb. It never touches the playground: the routes apply a direction through the
//! same `apply_scene_change` as `POST /dev/playground/scene`.

use std::collections::BTreeSet;
use std::sync::Arc;

use df_playground::scene::SceneChange;
use df_table::director::reason::{MAXIMUM_CAMERA_MS, MINIMUM_ZOOM_PERCENT, Reason};
use df_table::director::view::{BeatHold, BeatKind, PlateView, StageView, TargetView};
use df_table::director::{Director, DirectorSettings, Request, StageDirection};
use df_table::element::ElementId;
use df_table::inspect::InspectAudience;
use df_table::ports::Clock;

use super::limits::SHOWN_LOG_ENTRIES;
use super::request::{DirectorAct, DirectorError, ShowReply, StartKind, ViewEdit, ViewSummary};

/// What a verb decided: the moves to apply and the lines to write.
#[derive(Debug, Default)]
pub struct Decided {
    pub directions: Vec<StageDirection>,
    pub lines: Vec<String>,
}

/// The operator's edits to what the Director sees.
#[derive(Debug, Clone, Default)]
struct ViewEdits {
    hidden: BTreeSet<ElementId>,
    heroes: BTreeSet<ElementId>,
    card: bool,
    width_px: Option<u32>,
    stage_px: u32,
    collapsed_beat: bool,
    reduced_effects: bool,
    beat: Option<BeatHold>,
}

pub struct Workbench {
    director: Director,
    clock: Arc<dyn Clock>,
    /// What `director advance` added to the clock's time.
    skew_ms: u64,
    audience: InspectAudience,
    stage: Vec<ElementId>,
    /// The stage names the last refresh could not make into element names.
    refused: Vec<String>,
    edits: ViewEdits,
}

/// The camera change a direction is, with the numbers the dev camera verbs check.
pub fn scene_change(direction: &StageDirection) -> SceneChange {
    match direction {
        StageDirection::Focus {
            element,
            zoom_percent,
            over_ms,
            ..
        } => SceneChange::Camera {
            target: Some(element.to_string()),
            zoom_percent: *zoom_percent,
            duration_ms: *over_ms,
        },
        StageDirection::Wide { over_ms, .. } => SceneChange::Camera {
            target: None,
            zoom_percent: MINIMUM_ZOOM_PERCENT,
            duration_ms: *over_ms,
        },
    }
}

/// `director show`'s words for the plate.
fn plate_text(plate: &PlateView) -> String {
    match plate {
        PlateView::Card => "card (no plate, no anchors)".to_owned(),
        PlateView::Plate {
            width_px: Some(width),
            ..
        } => format!("plate {width} px wide"),
        PlateView::Plate { width_px: None, .. } => "plate of unknown width".to_owned(),
    }
}

impl Workbench {
    /// A workbench reading `clock`. `audience` says who reads: a local build is the operator.
    pub fn new(audience: InspectAudience, clock: Arc<dyn Clock>) -> Self {
        Self {
            director: Director::new(DirectorSettings::default()),
            clock,
            skew_ms: 0,
            audience,
            stage: Vec::new(),
            refused: Vec::new(),
            edits: ViewEdits::default(),
        }
    }

    /// The clock's time plus what `advance` added, never earlier than the Director last saw.
    fn now_ms(&self) -> u64 {
        self.clock
            .now_ms()
            .saturating_add(self.skew_ms)
            .max(self.director.now_ms())
    }

    /// Takes the names of the elements on the stage now. A name that is not an element name
    /// cannot be a target and is left out; the audit line that says so is returned once for each
    /// change of the names left out (the names themselves only to the operator).
    pub fn refresh_stage(&mut self, ids: &[String]) -> Option<String> {
        let mut stage = Vec::new();
        let mut refused = Vec::new();
        for id in ids {
            match ElementId::new(id) {
                Ok(element) => stage.push(element),
                Err(_) => refused.push(id.clone()),
            }
        }
        self.stage = stage;
        if refused == self.refused {
            return None;
        }
        self.refused = refused;
        let names = if self.audience.is_operator() {
            format!(" names={:?}", self.refused)
        } else {
            String::new()
        };
        (!self.refused.is_empty()).then(|| {
            format!(
                "CAMERA stage_names_refused count={}{names}",
                self.refused.len()
            )
        })
    }

    /// Whether the Director has anything to decide on a timer: a focus held, a request waiting or
    /// a pull-back to make again.
    pub fn is_busy(&self) -> bool {
        self.director.has_work()
    }

    /// The host could not put a move on the screen; the Director forgets or repeats it.
    pub fn apply_failed(&mut self, direction: &StageDirection) {
        self.director.apply_failed(direction);
    }

    /// The camera was moved by something other than the Director and now looks at `target` (an
    /// element name) or at the whole stage. Returns whether a focus was forgotten.
    pub fn camera_moved_elsewhere(&mut self, target: Option<&str>) -> Result<bool, DirectorError> {
        // A camera may look at a region, whose name is no element name: the Director then holds
        // no element, which is what `None` tells it.
        let target = target.and_then(|name| ElementId::new(name).ok());
        Ok(self
            .director
            .camera_moved_elsewhere(self.now_ms(), target.as_ref())?)
    }

    /// The Director itself, for a test that asks it directly as the engine will.
    #[cfg(test)]
    pub(super) fn director_mut(&mut self) -> &mut Director {
        &mut self.director
    }

    /// The shared screen as the Director sees it: the stage's elements, each as the edits say.
    pub fn view(&self) -> StageView {
        let plate = if self.edits.card {
            PlateView::Card
        } else {
            PlateView::Plate {
                generation: 1,
                width_px: self.edits.width_px,
            }
        };
        let mut view = StageView::new(plate);
        view.stage_width_px = self.edits.stage_px;
        view.collapsed_beat = self.edits.collapsed_beat;
        view.reduced_effects = self.edits.reduced_effects;
        view.beat = self.edits.beat.clone();
        for id in &self.stage {
            let target = TargetView {
                visible: !self.edits.hidden.contains(id),
                hero: self.edits.heroes.contains(id),
                anchored: true,
            };
            view = view.with_target(id.clone(), target);
        }
        view
    }

    fn collect(&mut self, directions: Vec<StageDirection>) -> Decided {
        let audience = self.audience;
        let lines = self
            .director
            .take_unread()
            .iter()
            .flat_map(|entry| entry.lines(audience))
            .collect();
        Decided { directions, lines }
    }

    /// The Director's own decision for the time now, with nothing asked: a hold that ran out, a
    /// target that was hidden, a waiting direction whose turn came.
    pub fn tick(&mut self) -> Result<Decided, DirectorError> {
        let view = self.view();
        let moves = self.director.tick(self.now_ms(), &view)?;
        Ok(self.collect(moves))
    }

    fn edit_view(&mut self, edit: ViewEdit) {
        match edit {
            ViewEdit::Hide { element } => {
                self.edits.hidden.insert(element);
            }
            ViewEdit::Unhide { element } => {
                self.edits.hidden.remove(&element);
            }
            ViewEdit::Hero { element } => {
                self.edits.heroes.insert(element);
            }
            ViewEdit::Card => self.edits.card = true,
            ViewEdit::Plate { width_px, stage_px } => {
                self.edits.card = false;
                self.edits.width_px = width_px;
                if let Some(stage_px) = stage_px {
                    self.edits.stage_px = stage_px;
                }
            }
            ViewEdit::Collapsed { on } => self.edits.collapsed_beat = on,
            ViewEdit::Reduced { on } => self.edits.reduced_effects = on,
            ViewEdit::Beat {
                reveal,
                hold_ms,
                element,
            } => {
                self.edits.beat = Some(BeatHold {
                    kind: if reveal {
                        BeatKind::RevealBeat
                    } else {
                        BeatKind::ResultCard
                    },
                    until_ms: self.now_ms().saturating_add(hold_ms),
                    subject: element,
                });
            }
            ViewEdit::Clear => self.edits = ViewEdits::default(),
        }
    }

    /// Runs one verb and then lets the Director decide for this tick, as the engine will each
    /// host tick: the moves to apply and the audit lines, in the order they were made.
    pub fn act(&mut self, act: DirectorAct) -> Result<Decided, DirectorError> {
        let mut moves = Vec::new();
        match act {
            DirectorAct::Focus {
                element,
                reason,
                zoom_percent,
                over_ms,
                hold_ms,
            } => {
                let reason = reason.unwrap_or(Reason::Speaker);
                let mut request = Request::focus(reason, element).driven();
                request.zoom_percent = zoom_percent;
                request.over_ms = over_ms;
                request.hold_ms = hold_ms;
                let view = self.view();
                self.director.request(self.now_ms(), &view, request)?;
            }
            DirectorAct::Wide { over_ms } => {
                let view = self.view();
                moves.push(self.director.pull_back(self.now_ms(), &view, over_ms)?);
            }
            DirectorAct::Pause => self.director.set_paused(true),
            DirectorAct::Resume => self.director.set_paused(false),
            DirectorAct::Auto { on } => self.director.set_auto_camera(on),
            DirectorAct::Advance { by_ms } => {
                if by_ms > MAXIMUM_CAMERA_MS {
                    return Err(DirectorError::InvalidAdvance { by_ms });
                }
                self.skew_ms = self.skew_ms.saturating_add(by_ms);
            }
            DirectorAct::End { reason, element } => {
                self.director.end(self.now_ms(), reason, &element)?;
            }
            DirectorAct::Start { what } => {
                let now = self.now_ms();
                let direction = match what {
                    StartKind::Scene => self.director.start_scene(now)?,
                    StartKind::Combat => self.director.start_combat(now, &self.view())?,
                };
                moves.extend(direction);
            }
            DirectorAct::View { edit } => self.edit_view(edit),
        }
        let view = self.view();
        moves.extend(self.director.tick(self.now_ms(), &view)?);
        Ok(self.collect(moves))
    }

    /// `director show`'s words for the held beat, naming its subject only to an audience that may
    /// be told it.
    fn beat_text(&self, view: &StageView) -> Option<String> {
        let beat = self.edits.beat.as_ref()?;
        let kind = match beat.kind {
            BeatKind::ResultCard => "result card",
            BeatKind::RevealBeat => "reveal beat",
        };
        let about = beat
            .subject
            .as_ref()
            .filter(|subject| {
                self.audience.is_operator()
                    || view
                        .target(subject)
                        .is_some_and(|target| target.focusable())
            })
            .map(|subject| format!(" about {subject}"))
            .unwrap_or_default();
        Some(format!("{kind}{about} until {} ms", beat.until_ms))
    }

    /// `director show`: the report for this workbench's audience and what the Director sees.
    pub fn show(&self) -> ShowReply {
        let view = self.view();
        let names = |set: &BTreeSet<ElementId>| set.iter().map(ToString::to_string).collect();
        ShowReply {
            report: self
                .director
                .report(self.audience, &view, SHOWN_LOG_ENTRIES),
            view: ViewSummary {
                plate: plate_text(&view.plate),
                stage_px: self.edits.stage_px,
                // What is hidden from the table is the operator's to read, never the shared
                // audience's.
                hidden: if self.audience.is_operator() {
                    names(&self.edits.hidden)
                } else {
                    Vec::new()
                },
                heroes: names(&self.edits.heroes),
                collapsed_beat: self.edits.collapsed_beat,
                reduced_effects: self.edits.reduced_effects,
                beat: self.beat_text(&view),
            },
        }
    }
}
