//! What the page remembers between draws so a redraw carries on instead of starting again: which
//! slide and which camera move it has begun, and when, and which host run they belong to.
//!
//! The host counts slides, camera moves, cues and changes from 0 each time it starts, so a counter
//! alone cannot say "the same move". A move is the pair of the host's boot id and its serial, and
//! when the boot id changes the page drops everything it remembers. The host also says how long ago
//! a move began (`elapsed_ms`), so a page that opens in the middle of one carries on from there.
//! Every time is a reading of a steady clock, in milliseconds, passed in: nothing here reads a
//! clock, so native tests cover it all.

use crate::camera::{Framing, framing, progress};
use crate::fixture::{Camera, Slide, StageElement};
use crate::limits::MAXIMUM_REMEMBERED_SLIDES;

/// Whether a controls answer should replace what the page has. `current` is the boot id and
/// revision of the answer the page last took. An answer from another run of the host always
/// replaces it, because its counters start again; from the same run, only a newer revision does, so
/// a slow answer that was overtaken never undoes a newer one.
pub fn accepts(current: Option<(&str, u64)>, boot_id: &str, revision: u64) -> bool {
    match current {
        None => true,
        Some((known_boot, known_revision)) => known_boot != boot_id || revision > known_revision,
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
    duration_ms: u32,
    from: Framing,
    to: Framing,
    /// The target went away after the move was made: the camera is back on the whole stage and
    /// stays there until a new move is made, even if the element comes back.
    released: bool,
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
    pub duration_ms: u32,
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
    /// A move that is already under way keeps its start and its clock, so the page follows a
    /// travelling target without restarting. A new move starts from where the last had got to, so
    /// an interrupted move pans on and does not pull out first. A move the page meets for the first
    /// time began `elapsed_ms` ago. A move whose target is not on the stage is over: the camera is
    /// on the whole stage and a respawn of the element does not zoom again; only a new move does.
    /// A camera from a scenario file (serial 0) follows its target whenever the target is there.
    pub fn plan(
        &mut self,
        now: f64,
        camera: Option<&Camera>,
        elements: Option<&[StageElement]>,
    ) -> CameraPlan {
        let Some(camera) = camera else {
            self.run = None;
            return CameraPlan::resting(Framing::WHOLE_STAGE, 0);
        };
        let Some(elements) = elements else {
            return CameraPlan::resting(Framing::WHOLE_STAGE, camera.serial);
        };
        let target_there = camera
            .target
            .as_deref()
            .is_none_or(|target| elements.iter().any(|element| element.id == target));
        let gone = camera.serial != 0 && !target_there;
        let mut run = match self.run.take() {
            Some(run) if run.serial == camera.serial => run,
            previous => CameraRun {
                serial: camera.serial,
                began: now - camera.elapsed_ms as f64,
                duration_ms: camera.duration_ms,
                from: previous.map_or(Framing::WHOLE_STAGE, |known| known.at(now)),
                to: Framing::WHOLE_STAGE,
                released: false,
            },
        };
        run.released |= gone;
        run.to = if run.released {
            Framing::WHOLE_STAGE
        } else {
            framing(Some(camera), elements)
        };
        let elapsed_ms = (now - run.began).max(0.0);
        let plan = if run.released || progress(elapsed_ms, run.duration_ms) >= 1.0 {
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
    ) -> CameraPlan {
        self.camera.plan(now, camera, elements)
    }
}
