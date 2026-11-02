//! Plain geometry shared by the stage and its tests: canvas units to percent, health to a bar.
//!
//! It never draws and never reads a fixture.

/// Width of the canvas in canvas units.
pub const CANVAS_WIDTH: f32 = 100.0;
/// Height of the canvas in canvas units (a 16 by 9 screen).
pub const CANVAS_HEIGHT: f32 = 56.25;

/// Where a point of the canvas sits on the stage, as percent from the left and from the top.
/// A point outside the canvas is held on its edge, so a stray element stays visible.
pub fn stage_percent(x: f32, y: f32) -> (f32, f32) {
    let clamp = |value: f32, extent: f32| {
        if value.is_finite() {
            (value / extent * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        }
    };
    (clamp(x, CANVAS_WIDTH), clamp(y, CANVAS_HEIGHT))
}

/// Size of an element on the stage, as percent of the stage's width and height.
pub fn stage_size_percent(width: f32, height: f32) -> (f32, f32) {
    let percent = |value: f32, extent: f32| {
        if value.is_finite() && value > 0.0 {
            (value / extent * 100.0).min(100.0)
        } else {
            0.0
        }
    };
    (percent(width, CANVAS_WIDTH), percent(height, CANVAS_HEIGHT))
}

/// A health bar's fill, in thousandths of the bar. Zero maximum gives an empty bar, and health
/// above the maximum fills it.
pub fn hit_point_permille(hit_points: u32, maximum: u32) -> u16 {
    if maximum == 0 {
        return 0;
    }
    let permille = u64::from(hit_points.min(maximum)) * 1_000 / u64::from(maximum);
    u16::try_from(permille).unwrap_or(1_000)
}
