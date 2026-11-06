//! Where the shared screen's camera looks, as plain numbers.
//!
//! The camera frames the world layer (the backdrop, the figures and the card) by scaling it about
//! its top-left corner and sliding it so the target sits in the middle of the screen. The slide is
//! held so the picture always covers the screen: a target near an edge is as near the middle as the
//! edge allows. It never draws; `view::tv` puts these numbers into a style.

use crate::controls::ZoomLimit;
use crate::fixture::{Camera, StageElement};
use crate::layout::stage_percent;
use crate::limits::{MAXIMUM_ZOOM_PERCENT, MINIMUM_ZOOM_PERCENT};

/// The position and size of the world layer for one framing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Framing {
    /// How many times larger than the whole stage the world is drawn; 1 is the whole stage.
    pub scale: f32,
    /// How far the world is moved left (negative) and up (negative), as percent of the stage.
    pub x_percent: f32,
    pub y_percent: f32,
}

impl Framing {
    /// The whole stage, with nothing moved.
    pub const WHOLE_STAGE: Self = Self {
        scale: 1.0,
        x_percent: 0.0,
        y_percent: 0.0,
    };

    /// The CSS transform that draws this framing. Numbers only, so no text from a scene file can
    /// reach a style.
    pub fn transform(&self) -> String {
        format!(
            "translate({:.3}%, {:.3}%) scale({:.4})",
            self.x_percent, self.y_percent, self.scale
        )
    }

    /// The framing part-way from `self` to `to`, `progress` being 0 at `self` and 1 at `to`. The
    /// page's animation moves each number evenly, and so does this.
    pub fn blend(&self, to: &Self, progress: f32) -> Self {
        let progress = progress.clamp(0.0, 1.0);
        let step = |from: f32, to: f32| from + (to - from) * progress;
        Self {
            scale: step(self.scale, to.scale),
            x_percent: step(self.x_percent, to.x_percent),
            y_percent: step(self.y_percent, to.y_percent),
        }
    }
}

/// How far a move of `duration_ms` has gone after `elapsed_ms`, from 0 to 1. A move that takes no
/// time is done at once.
pub fn progress(elapsed_ms: f64, duration_ms: u64) -> f32 {
    if duration_ms == 0 {
        return 1.0;
    }
    (elapsed_ms / duration_ms as f64).clamp(0.0, 1.0) as f32
}

/// The framing a camera asks for with the elements now on the stage. A camera without a target, a
/// target that is not on the stage, or a zoom of 100 percent shows the whole stage.
pub fn framing(camera: Option<&Camera>, elements: &[StageElement]) -> Framing {
    framing_within(camera, elements, MAXIMUM_ZOOM_PERCENT)
}

/// The zoom a picture can bear, in percent: the least of the largest zoom the model allows and the
/// plate's width in pixels over the stage's width in device pixels (a plate drawn larger than its
/// own pixels is a blur), and never less than the whole stage. With no plate known, or a stage of
/// no width, only the model's largest applies.
pub fn zoom_ceiling_percent(plate_width_px: Option<u32>, stage_width_px: u32) -> u16 {
    let (Some(plate), true) = (plate_width_px, stage_width_px > 0) else {
        return MAXIMUM_ZOOM_PERCENT;
    };
    let bearable = u64::from(plate) * 100 / u64::from(stage_width_px);
    let floor = u64::from(MINIMUM_ZOOM_PERCENT);
    let ceiling = u64::from(MAXIMUM_ZOOM_PERCENT);
    u16::try_from(bearable.clamp(floor, ceiling)).unwrap_or(MAXIMUM_ZOOM_PERCENT)
}

/// The zoom ceiling under `limit`: the model's largest, or the picture's own pixels
/// (`zoom_ceiling_percent`).
pub fn zoom_ceiling_under(
    limit: ZoomLimit,
    plate_width_px: Option<u32>,
    stage_width_px: u32,
) -> u16 {
    match limit {
        ZoomLimit::Model => MAXIMUM_ZOOM_PERCENT,
        ZoomLimit::Pixels => zoom_ceiling_percent(plate_width_px, stage_width_px),
    }
}

/// The zoom that is drawn when `requested` is asked for under `ceiling`, and the ceiling itself
/// when the request was cut (so the page can say so and not cut it silently).
pub fn clamped_zoom(requested: u16, ceiling: u16) -> (u16, Option<u16>) {
    if requested > ceiling {
        (ceiling, Some(ceiling))
    } else {
        (requested, None)
    }
}

/// `framing` with the zoom held to `zoom_ceiling` percent.
pub fn framing_within(
    camera: Option<&Camera>,
    elements: &[StageElement],
    zoom_ceiling: u16,
) -> Framing {
    let Some(camera) = camera else {
        return Framing::WHOLE_STAGE;
    };
    let Some(target) = camera.target.as_deref() else {
        return Framing::WHOLE_STAGE;
    };
    let Some(item) = elements.iter().find(|item| item.id == target) else {
        return Framing::WHOLE_STAGE;
    };
    let scale = f32::from(clamped_zoom(camera.zoom_percent, zoom_ceiling).0) / 100.0;
    if scale <= 1.0 {
        return Framing::WHOLE_STAGE;
    }
    let (left, top) = stage_percent(item.x, item.y);
    // The least the world may move is enough to keep its far edge on the screen's far edge.
    let hold = |centre: f32| (50.0 - centre * scale).clamp(100.0 - 100.0 * scale, 0.0);
    Framing {
        scale,
        x_percent: hold(left),
        y_percent: hold(top),
    }
}
