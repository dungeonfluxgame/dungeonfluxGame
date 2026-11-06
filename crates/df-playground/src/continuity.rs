//! What the page remembers between draws so a redraw carries on instead of starting again: which
//! slide and which camera move it has begun, and when, and which host run they belong to.
//!
//! The host counts slides, camera moves, cues and changes from 0 each time it starts, so a counter
//! alone cannot say "the same move". A move is the pair of the host's boot id and its serial, and
//! when the boot id changes the page drops everything it remembers. The host also says how long ago
//! a move began (`elapsed_ms`), so a page that opens in the middle of one carries on from there.
//! Every time is a reading of a steady clock, in milliseconds, passed in: nothing here reads a
//! clock, so native tests cover it all.

use crate::camera::{Framing, framing_within, progress};
use crate::fixture::{Camera, Slide, StageElement};
use crate::limits::{MAXIMUM_REMEMBERED_SLIDES, MAXIMUM_ZOOM_PERCENT, OLDER_RUN_STRIKES};

/// What the page does with an answer from the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Take it; `new_run` says it is from another run of the host than the one followed.
    Take { new_run: bool },
    /// Leave it: an older answer than one already taken.
    Refuse { older_run: bool },
}

/// Decides which answers from the host the page takes, so a slow answer that was overtaken never
/// undoes a newer one. A boot id starts with the time the host started in fixed-width hex, so ids
/// are ordered: an answer from an older run than the one followed is refused, and from the same
/// run only a newer revision is taken. Many refusals of one older run in a row mean that run is
/// the real one (the host's clock was set back), and it is taken.
#[derive(Debug, Default)]
pub struct ReplyGate {
    current: Option<(String, u64)>,
    older: Option<(String, u8)>,
}

impl ReplyGate {
    pub fn admit(&mut self, boot_id: &str, revision: u64) -> Admit {
        let Some((known_boot, known_revision)) = &self.current else {
            self.take(boot_id, revision);
            return Admit::Take { new_run: false };
        };
        if boot_id == known_boot {
            // The run being followed has spoken, so a stray answer of an older one was only that.
            self.older = None;
            if revision > *known_revision {
                self.take(boot_id, revision);
                return Admit::Take { new_run: false };
            }
            return Admit::Refuse { older_run: false };
        }
        if boot_id > known_boot.as_str() {
            self.take(boot_id, revision);
            return Admit::Take { new_run: true };
        }
        let strikes = match &self.older {
            Some((boot, strikes)) if boot == boot_id => strikes + 1,
            _ => 1,
        };
        if strikes >= OLDER_RUN_STRIKES {
            self.take(boot_id, revision);
            return Admit::Take { new_run: true };
        }
        self.older = Some((boot_id.to_owned(), strikes));
        Admit::Refuse { older_run: true }
    }

    fn take(&mut self, boot_id: &str, revision: u64) {
        self.current = Some((boot_id.to_owned(), revision));
        self.older = None;
    }

    /// The run of the host being followed.
    pub fn boot_id(&self) -> Option<&str> {
        self.current.as_ref().map(|(boot, _)| boot.as_str())
    }

    /// Whether any answer has been taken yet.
    pub fn is_started(&self) -> bool {
        self.current.is_some()
    }
}

/// Whether a canvas answer should replace the canvas the page has. `known` is the run and the
/// revision of that canvas (`None` when there is none); one from the same run is taken only when
/// newer, so a slow read of the canvas never undoes a frame that arrived meanwhile.
pub fn canvas_accepts(known: Option<(&str, u64)>, boot_id: &str, revision: u64) -> bool {
    match known {
        None => true,
        Some((known_boot, known_revision)) => {
            boot_id > known_boot || (boot_id == known_boot && revision > known_revision)
        }
    }
}

/// Whether two slides are the same one. A slide the host numbered is told by its number; one from
/// a scenario file has no number and is told by what it says.
fn same_slide(known: &Slide, slide: &Slide) -> bool {
    known.serial == slide.serial
        && (slide.serial != 0
            || (known.from_percent, known.to_percent, known.duration_ms)
                == (slide.from_percent, slide.to_percent, slide.duration_ms))
}

/// The slides this page has begun, each with the reading of the clock when it began.
#[derive(Debug, Default)]
pub struct SlideClock {
    slides: Vec<(Slide, f64)>,
}

impl SlideClock {
    /// How long ago this slide began, in milliseconds. The first time a slide is asked about, it
    /// began `elapsed_ms` ago (what the host said), so a page that opens mid-slide carries on from
    /// there.
    pub fn elapsed(&mut self, now: f64, slide: &Slide) -> f64 {
        if let Some((_, began)) = self
            .slides
            .iter()
            .find(|(known, _)| same_slide(known, slide))
        {
            return (now - began).max(0.0);
        }
        if self.slides.len() >= MAXIMUM_REMEMBERED_SLIDES {
            self.slides.remove(0);
        }
        let told = slide.elapsed_ms as f64;
        self.slides.push((*slide, now - told));
        told
    }

    /// Forgets every slide, so the next scene's slides begin from their start.
    pub fn forget(&mut self) {
        self.slides.clear();
    }

    pub fn len(&self) -> usize {
        self.slides.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slides.is_empty()
    }
}

/// The camera move this page is part-way through, so a redraw carries on from where the picture
/// was instead of jumping, and a new move starts from where the last one had got to.
#[derive(Debug, Clone, PartialEq)]
struct CameraRun {
    serial: u64,
    began: f64,
    duration_ms: u64,
    from: Framing,
    to: Framing,
}

impl CameraRun {
    fn at(&self, now: f64) -> Framing {
        self.from
            .blend(&self.to, progress(now - self.began, self.duration_ms))
    }
}

/// How the world layer is framed for one draw, and what the draw needs to know about the move.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPlan {
    /// Where the move started from.
    pub from: Framing,
    /// Where it ends, or where the camera rests when it is not moving.
    pub to: Framing,
    /// How long the move has been going, in milliseconds.
    pub elapsed_ms: f64,
    pub duration_ms: u64,
    /// Whether the move is still under way.
    pub moving: bool,
    /// Which move this is; 0 for none.
    pub serial: u64,
}

impl CameraPlan {
    fn resting(to: Framing, serial: u64) -> Self {
        Self {
            from: to,
            to,
            elapsed_ms: 0.0,
            duration_ms: 0,
            moving: false,
            serial,
        }
    }
}

/// The camera moves this page has begun.
#[derive(Debug, Default)]
pub struct CameraClock {
    run: Option<CameraRun>,
}

impl CameraClock {
    /// The framing to draw for `camera` with the `elements` now on the stage, at clock reading
    /// `now`. `elements` is `None` while the page does not know the stage yet, which draws the
    /// whole stage and changes nothing the page remembers.
    ///
    /// The framing is worked out from what the frame says and nothing the page saw earlier, so
    /// every page that has the same frame draws the same picture: a target that is not on the
    /// stage shows the whole stage, and one that is shows the zoom. A move that is already under
    /// way keeps its start and its clock, so the page follows a travelling target without
    /// restarting. A new move starts from where the last had got to, so an interrupted move pans
    /// on and does not pull out first. A move the page meets for the first time began
    /// `elapsed_ms` ago.
    pub fn plan(
        &mut self,
        now: f64,
        camera: Option<&Camera>,
        elements: Option<&[StageElement]>,
    ) -> CameraPlan {
        self.plan_within(now, camera, elements, MAXIMUM_ZOOM_PERCENT)
    }

    /// `plan` with the zoom, in percent, that the picture can bear as the ceiling.
    pub fn plan_within(
        &mut self,
        now: f64,
        camera: Option<&Camera>,
        elements: Option<&[StageElement]>,
        zoom_ceiling: u16,
    ) -> CameraPlan {
        let Some(camera) = camera else {
            self.run = None;
            return CameraPlan::resting(Framing::WHOLE_STAGE, 0);
        };
        let Some(elements) = elements else {
            return CameraPlan::resting(Framing::WHOLE_STAGE, camera.serial);
        };
        let mut run = match self.run.take() {
            Some(run) if run.serial == camera.serial => run,
            previous => CameraRun {
                serial: camera.serial,
                began: now - camera.elapsed_ms as f64,
                duration_ms: camera.duration_ms,
                from: previous.map_or(Framing::WHOLE_STAGE, |known| known.at(now)),
                to: Framing::WHOLE_STAGE,
            },
        };
        run.to = framing_within(Some(camera), elements, zoom_ceiling);
        let elapsed_ms = (now - run.began).max(0.0);
        let plan = if progress(elapsed_ms, run.duration_ms) >= 1.0 {
            CameraPlan::resting(run.to, run.serial)
        } else {
            CameraPlan {
                from: run.from,
                to: run.to,
                elapsed_ms,
                duration_ms: run.duration_ms,
                moving: true,
                serial: run.serial,
            }
        };
        self.run = Some(run);
        plan
    }

    pub fn forget(&mut self) {
        self.run = None;
    }
}

/// Everything the page remembers about the moves of one run of the host.
#[derive(Debug, Default)]
pub struct Continuity {
    boot_id: Option<String>,
    slides: SlideClock,
    camera: CameraClock,
}

impl Continuity {
    /// Notes the host run an answer came from. When it is not the run the page was following,
    /// everything remembered is dropped, because the counters it was keyed by started again.
    /// Returns whether that happened.
    pub fn note_boot(&mut self, boot_id: &str) -> bool {
        match self.boot_id.as_deref() {
            Some(known) if known == boot_id => false,
            known => {
                let replaced = known.is_some();
                self.boot_id = Some(boot_id.to_owned());
                if replaced {
                    self.forget();
                }
                replaced
            }
        }
    }

    /// Forgets every slide and camera move, so the next scene starts from its own beginning.
    pub fn forget(&mut self) {
        self.slides.forget();
        self.camera.forget();
    }

    pub fn slide_elapsed(&mut self, now: f64, slide: &Slide) -> f64 {
        self.slides.elapsed(now, slide)
    }

    pub fn camera_plan(
        &mut self,
        now: f64,
        camera: Option<&Camera>,
        elements: Option<&[StageElement]>,
        zoom_ceiling: u16,
    ) -> CameraPlan {
        self.camera.plan_within(now, camera, elements, zoom_ceiling)
    }
}
