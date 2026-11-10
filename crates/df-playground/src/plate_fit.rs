//! Where a plate (a picture of any shape) lies in the shared screen's 16 by 9 frame, and where a
//! point of the picture is on the screen.
//!
//! A scan's regions are in thousandths of the picture. The screen is a 16 by 9 frame, and a picture
//! of another shape is not drawn edge to edge in it, so a point of the picture is a different point
//! of the screen. This is the one place that knows that: every item tied to the picture (the
//! speaker's outline and mask, the thinking cue, the bubbles' tails, the name tags, the faces the
//! bubbles and the clue board keep clear of, the camera's region targets) takes its screen place
//! from [`PlateFit::stage_permille`] and its kin, and none keeps a copy of the arithmetic.
//!
//! The fit policy is one rule. A picture whose shape is within [`COVER_TOLERANCE_PERCENT`] of the
//! frame's is cropped to fill it (cover), as plates always were. Any other is shown whole inside
//! the frame (contain) over a blurred, dimmed copy of itself, so nobody in the picture is ever cut
//! away: a square or a tall plate is never cropped to a band of its middle. The policy is fixed
//! here and not a control: the page decides nothing the table could set.

use crate::image_layout::PictureBinding;
use crate::layout::{CANVAS_HEIGHT, CANVAS_WIDTH};

/// How far, in percent, a picture's shape may differ from the frame's and still be cropped to fill
/// it. A picture further off is shown whole.
pub const COVER_TOLERANCE_PERCENT: f64 = 5.0;

/// The marker of a plate's fit on its element, which the theme reads (`data-fit`).
pub const FIT_ATTRIBUTE: &str = "data-fit";

/// How a plate fills the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// The picture is cropped (a little) to fill the frame.
    Cover,
    /// The whole picture lies inside the frame, with the blurred copy of it round it.
    Contain,
}

impl Fit {
    /// The word the theme's `data-fit` is written with.
    pub fn key(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Contain => "contain",
        }
    }
}

/// The box the picture occupies in the frame, as fractions of the frame's width and height. Under
/// cover the box is larger than the frame (the crop), under contain smaller on one axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlateFit {
    fit: Fit,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

impl PlateFit {
    /// A picture exactly the frame's shape: it is the frame.
    pub const FRAME: Self = Self {
        fit: Fit::Cover,
        left: 0.0,
        top: 0.0,
        width: 1.0,
        height: 1.0,
    };

    /// How a picture `width_px` by `height_px` lies in the frame. A picture with no size is taken
    /// to be the frame's shape.
    pub fn of_size(width_px: u32, height_px: u32) -> Self {
        if width_px == 0 || height_px == 0 {
            return Self::FRAME;
        }
        // The picture's shape over the frame's: above 1 the picture is the wider.
        let relative = f64::from(CANVAS_HEIGHT) * f64::from(width_px)
            / (f64::from(CANVAS_WIDTH) * f64::from(height_px));
        let fit = if (relative - 1.0).abs() * 100.0 <= COVER_TOLERANCE_PERCENT {
            Fit::Cover
        } else {
            Fit::Contain
        };
        // Whose edges meet the frame's: the width's when the picture is wider and shown whole, or
        // narrower and cropped; the height's otherwise.
        let fills_width = (relative >= 1.0) == (fit == Fit::Contain);
        let (width, height) = if fills_width {
            (1.0, 1.0 / relative)
        } else {
            (relative, 1.0)
        };
        let (width, height) = (width as f32, height as f32);
        Self {
            fit,
            left: (1.0 - width) / 2.0,
            top: (1.0 - height) / 2.0,
            width,
            height,
        }
    }

    /// How the picture a layout was scanned from lies in the frame.
    pub fn of_picture(picture: &PictureBinding) -> Self {
        Self::of_size(picture.width_px, picture.height_px)
    }

    pub fn fit(self) -> Fit {
        self.fit
    }

    /// The width the picture is drawn at, as a fraction of the frame's width.
    pub fn width(self) -> f32 {
        self.width
    }

    /// The screen place of a point of the picture (`x`, `y` in thousandths of the picture), as
    /// fractions of the frame. A point of the cropped part is outside 0 to 1.
    pub fn stage_fraction(self, (x_permille, y_permille): (u16, u16)) -> (f32, f32) {
        (
            self.left + f32::from(x_permille) / 1000.0 * self.width,
            self.top + f32::from(y_permille) / 1000.0 * self.height,
        )
    }

    /// The screen place of a point of the picture in thousandths of the frame, held on the frame.
    pub fn stage_permille(self, point: (u16, u16)) -> (u16, u16) {
        let (x, y) = self.stage_fraction(point);
        (held_permille(x), held_permille(y))
    }

    /// The screen place of a point of the picture in the stage's canvas units.
    pub fn stage_units(self, point: (u16, u16)) -> (f32, f32) {
        let (x, y) = self.stage_fraction(point);
        (x * CANVAS_WIDTH, y * CANVAS_HEIGHT)
    }

    /// The screen box of a box of the picture (`left`, `top`, `right`, `bottom` in thousandths of
    /// the picture), in thousandths of the frame, held on the frame.
    pub fn stage_box_permille(
        self,
        (left, top, right, bottom): (u16, u16, u16, u16),
    ) -> (u16, u16, u16, u16) {
        let (left, top) = self.stage_permille((left, top));
        let (right, bottom) = self.stage_permille((right, bottom));
        (left, top, right, bottom)
    }

    /// The style that puts an element at the picture's box in the frame, so what is in it in
    /// percent of the picture (a mask, a scan overlay) is where the picture is. Numbers only.
    pub fn box_style(self) -> String {
        format!(
            "left:{:.3}%;top:{:.3}%;width:{:.3}%;height:{:.3}%;",
            self.left * 100.0,
            self.top * 100.0,
            self.width * 100.0,
            self.height * 100.0
        )
    }
}

fn held_permille(fraction: f32) -> u16 {
    (fraction * 1000.0).round().clamp(0.0, 1000.0) as u16
}
