//! The Director on the host: its controlled clock, the stand-in view of the shared screen and the
//! translation of a verb into the Director's calls (O.14, G1).
//!
//! The workbench is synchronous bookkeeping around `df_table::director::Director`. It reads the
//! time from a clock it owns (real elapsed time plus whatever `director advance` added, or only
//! what `advance` added in a manual clock), builds the `StageView` the Director decides on from
//! the stage's element names and the operator's view edits, and returns the directions and audit
//! lines of one verb. It never touches the playground: the routes apply a direction through the
//! same `apply_scene_change` as `POST /dev/playground/scene`.

use std::collections::BTreeSet;
use tokio::time::Instant;

use df_playground::scene::SceneChange;
use df_table::director::{Director, DirectorSettings, Request, StageDirection};
use df_table::director_reason::{MAXIMUM_CAMERA_MS, MINIMUM_ZOOM_PERCENT, Reason};
use df_table::director_view::{BeatHold, BeatKind, PlateView, StageView, TargetView};
use df_table::element::ElementId;
use df_table::inspect::InspectAudience;

use super::request::{DirectorAct, DirectorFault, ShowReply, StartKind, ViewEdit, ViewSummary};

/// Entries `director show` prints, newest last.
pub const SHOWN_LOG_ENTRIES: usize = 40;

/// Where the Director's time comes from.
#[derive(Debug)]
enum Clock {
    /// Elapsed real time, so a hold runs out by itself.
    Real(Instant),
    /// Only `advance` moves it: for tests that must not depend on time.
    Manual,
}

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
    clock: Clock,
    skew_ms: u64,
    audience: InspectAudience,
    stage: Vec<ElementId>,
    /// The stage names the last refresh could not make into element names.
    refused: Vec<String>,
    edits: ViewEdits,
}

fn element(text: &str) -> Result<ElementId, DirectorFault> {
    ElementId::new(text).map_err(|_| DirectorFault::BadElement {
        text: text.to_owned(),
    })
}

fn reason(text: &str) -> Result<Reason, DirectorFault> {
    Reason::from_key(text).ok_or_else(|| DirectorFault::UnknownReason {
        text: text.to_owned(),
    })
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

impl Workbench {
    /// A workbench on the real clock. `audience` says who reads: a local build is the operator.
    pub fn new(audience: InspectAudience) -> Result<Self, DirectorFault> {
        Self::build(audience, Clock::Real(Instant::now()))
    }

    /// A workbench whose clock only `advance` moves.
    pub fn manual(audience: InspectAudience) -> Result<Self, DirectorFault> {
        Self::build(audience, Clock::Manual)
    }

    fn build(audience: InspectAudience, clock: Clock) -> Result<Self, DirectorFault> {
        Ok(Self {
            director: Director::new(DirectorSettings::defaults())?,
            clock,
            skew_ms: 0,
            audience,
            stage: Vec::new(),
            refused: Vec::new(),
            edits: ViewEdits::default(),
        })
    }

    fn now_ms(&self) -> u64 {
        let elapsed = match &self.clock {
            Clock::Real(started) => {
                u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
            }
            Clock::Manual => 0,
        };
        elapsed
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
    pub fn camera_moved_elsewhere(&mut self, target: Option<&str>) -> Result<bool, DirectorFault> {
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
    pub fn tick(&mut self) -> Result<Decided, DirectorFault> {
        let view = self.view();
        let moves = self.director.tick(self.now_ms(), &view)?;
        Ok(self.collect(moves))
    }

    fn edit_view(&mut self, edit: ViewEdit) -> Result<(), DirectorFault> {
        match edit {
            ViewEdit::Hide { element: name } => {
                self.edits.hidden.insert(element(&name)?);
            }
            ViewEdit::Unhide { element: name } => {
                self.edits.hidden.remove(&element(&name)?);
            }
            ViewEdit::Hero { element: name } => {
                self.edits.heroes.insert(element(&name)?);
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
                ms,
                element: name,
            } => {
                let subject = name.as_deref().map(element).transpose()?;
                self.edits.beat = Some(BeatHold {
                    kind: if reveal {
                        BeatKind::RevealBeat
                    } else {
                        BeatKind::ResultCard
                    },
                    until_ms: self.now_ms().saturating_add(ms),
                    subject,
                });
            }
            ViewEdit::Clear => self.edits = ViewEdits::default(),
        }
        Ok(())
    }

    /// Runs one verb and then lets the Director decide for this tick, as the engine will each
    /// host tick: the moves to apply and the audit lines, in the order they were made.
    pub fn act(&mut self, act: DirectorAct) -> Result<Decided, DirectorFault> {
        let mut moves = Vec::new();
        match act {
            DirectorAct::Focus {
                element: name,
                reason: reason_key,
                zoom,
                over,
                hold,
            } => {
                let reason = reason_key.as_deref().map_or(Ok(Reason::Speaker), reason)?;
                let mut request = Request::focus(reason, element(&name)?).driven();
                request.zoom_percent = zoom;
                request.over_ms = over;
                request.hold_ms = hold;
                let view = self.view();
                self.director.request(self.now_ms(), &view, request)?;
            }
            DirectorAct::Wide { over } => {
                let view = self.view();
                moves.push(self.director.pull_back(self.now_ms(), &view, over)?);
            }
            DirectorAct::Pause => self.director.set_paused(true),
            DirectorAct::Resume => self.director.set_paused(false),
            DirectorAct::Auto { on } => self.director.set_auto_camera(on),
            DirectorAct::Advance { ms } => {
                if ms > u64::from(MAXIMUM_CAMERA_MS) {
                    return Err(DirectorFault::Refused {
                        reason: format!("a clock advance is 0 to {MAXIMUM_CAMERA_MS} ms, not {ms}"),
                    });
                }
                self.skew_ms = self.skew_ms.saturating_add(ms);
            }
            DirectorAct::End {
                reason: key,
                element: name,
            } => {
                self.director
                    .end(self.now_ms(), reason(&key)?, &element(&name)?)?;
            }
            DirectorAct::Start { what } => {
                let now = self.now_ms();
                let direction = match what {
                    StartKind::Scene => self.director.start_scene(now)?,
                    StartKind::Combat => self.director.start_combat(now, &self.view())?,
                };
                moves.extend(direction);
            }
            DirectorAct::View { edit } => self.edit_view(edit)?,
        }
        let view = self.view();
        moves.extend(self.director.tick(self.now_ms(), &view)?);
        Ok(self.collect(moves))
    }

    /// `director show`: the report for this workbench's audience and what the Director sees.
    pub fn show(&self) -> ShowReply {
        let view = self.view();
        let names = |set: &BTreeSet<ElementId>| set.iter().map(ToString::to_string).collect();
        let plate = match view.plate {
            PlateView::Card => "card (no plate, no anchors)".to_owned(),
            PlateView::Plate { width_px, .. } => match width_px {
                Some(width) => format!("plate {width} px wide"),
                None => "plate of unknown width".to_owned(),
            },
        };
        let beat = self.edits.beat.as_ref().map(|beat| {
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
            format!("{kind}{about} until {} ms", beat.until_ms)
        });
        ShowReply {
            report: self
                .director
                .report(self.audience, &view, SHOWN_LOG_ENTRIES),
            view: ViewSummary {
                plate,
                stage_px: self.edits.stage_px,
                // What is hidden from the table is the operator's to read, never the shared audience's.
                hidden: if self.audience.is_operator() {
                    names(&self.edits.hidden)
                } else {
                    Vec::new()
                },
                heroes: names(&self.edits.heroes),
                collapsed_beat: self.edits.collapsed_beat,
                reduced_effects: self.edits.reduced_effects,
                beat,
            },
        }
    }
}
