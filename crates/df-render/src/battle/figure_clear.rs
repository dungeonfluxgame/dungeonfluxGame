//! Which scan splats may hide a figure.
//!
//! A scan's depth is the average of its splats' centre depths, and a splat is a flat disc whose
//! footprint on screen is far larger than its depth is accurate to: a tilted palisade seen along its
//! length, or any coarse level of detail, puts a splat whose centre is nearer than a figure over
//! the figure's body although the wood is beside it or behind it. The rule is therefore made on the
//! splat, not the pixel:
//!
//! - a splat whose centre lies in front of the figure (by [`FRONT_MARGIN_M`]) and projects *inside*
//!   the figure's screen box is truly between the camera and the figure: it hides what it covers;
//! - any other splat whose footprint touches the figure's box is beside it or behind it: it is
//!   drawn, but it adds nothing to the depth the figure is tested against.
//!
//! The GPU applies this per splat in the projection pass (`figure_clear` in `splat_shaders.rs`);
//! this module is the same rule in plain Rust, which the native tests run over the real scans.

use super::math::{Mat4, Vec3};

/// Figures protected at once (the party is going to six seats; both sides fit).
pub const MAX_FIGURES: usize = 16;
/// How much nearer than the figure a splat's centre must be to count as in front of it, metres.
pub const FRONT_MARGIN_M: f32 = 0.35;
/// The figure's screen box is this much larger than its body on every side (a share of its size).
pub const BOX_SLACK: f32 = 0.08;
/// Half the width of a figure's box as a share of its height (cloak, weapon and bow included).
pub const HALF_WIDTH_SHARE: f32 = 0.5;

/// Where a figure is on the screen: a box in normalised device coordinates and the view depth of
/// its middle in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FigureBox {
    /// Left, bottom, right, top (device y points up).
    pub rect: [f32; 4],
    pub depth_m: f32,
}

impl FigureBox {
    /// The values the projection shader reads for this figure.
    pub fn words(&self) -> [f32; 8] {
        [
            self.rect[0],
            self.rect[1],
            self.rect[2],
            self.rect[3],
            self.depth_m,
            1.0,
            0.0,
            0.0,
        ]
    }
}

/// The box of a figure standing at `feet` and `height_m` tall, seen through `view` and `view_proj`.
/// `None` when part of it is behind the camera.
pub fn figure_box(view: &Mat4, view_proj: &Mat4, feet: Vec3, height_m: f32) -> Option<FigureBox> {
    let look = Vec3::new(-view.0[2], 0.0, -view.0[10]);
    let right = Vec3::new(-look.z, 0.0, look.x).normalized();
    let half = height_m * HALF_WIDTH_SHARE;
    let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for side in [-1.0, 1.0] {
        for height in [0.0, height_m] {
            let point = feet + right * (half * side) + Vec3::new(0.0, height, 0.0);
            let (x, y, _) = view_proj.project(point)?;
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x);
            bounds[3] = bounds[3].max(y);
        }
    }
    let (width, height) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
    let rect = [
        bounds[0] - width * BOX_SLACK,
        bounds[1] - height * BOX_SLACK,
        bounds[2] + width * BOX_SLACK,
        bounds[3] + height * BOX_SLACK,
    ];
    let middle = feet + Vec3::new(0.0, height_m * 0.5, 0.0);
    let (_, _, depth_m) = view_proj.project(middle)?;
    Some(FigureBox { rect, depth_m })
}

/// Whether a splat takes no part in the depth that figures are tested against: it touches some
/// figure's box but is not truly between the camera and that figure, and is not truly between the
/// camera and any other.
///
/// `centre` is the splat's centre in normalised device coordinates, `reach` how far its footprint
/// reaches from the centre in the same units (per axis), `depth_m` its centre's view depth.
pub fn is_exempt(
    centre: (f32, f32),
    reach: (f32, f32),
    depth_m: f32,
    figures: &[FigureBox],
) -> bool {
    let mut touching = false;
    for figure in figures {
        let [left, bottom, right, top] = figure.rect;
        let inside = centre.0 >= left && centre.0 <= right && centre.1 >= bottom && centre.1 <= top;
        if inside && depth_m < figure.depth_m - FRONT_MARGIN_M {
            return false;
        }
        touching |= centre.0 + reach.0 > left
            && centre.0 - reach.0 < right
            && centre.1 + reach.1 > bottom
            && centre.1 - reach.1 < top;
    }
    touching
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::CameraPose;

    fn boxes() -> Vec<FigureBox> {
        vec![FigureBox {
            rect: [-0.1, -0.2, 0.1, 0.2],
            depth_m: 10.0,
        }]
    }

    #[test]
    fn a_splat_in_front_and_over_the_figure_hides_it() {
        assert!(!is_exempt((0.0, 0.0), (0.05, 0.05), 8.0, &boxes()));
    }

    #[test]
    fn a_big_splat_beside_the_figure_whose_centre_is_nearer_does_not() {
        // Its centre is outside the box and nearer, but its footprint reaches over the figure.
        assert!(is_exempt((0.3, 0.0), (0.3, 0.2), 8.0, &boxes()));
    }

    #[test]
    fn a_splat_behind_the_figure_takes_no_part() {
        assert!(is_exempt((0.0, 0.0), (0.05, 0.05), 12.0, &boxes()));
        // Too little in front to count as in front: ground and grass at the feet.
        assert!(is_exempt((0.0, -0.15), (0.05, 0.05), 9.8, &boxes()));
    }

    #[test]
    fn a_splat_that_does_not_touch_any_figure_is_left_alone() {
        assert!(!is_exempt((0.8, 0.5), (0.05, 0.05), 8.0, &boxes()));
    }

    #[test]
    fn a_figures_box_surrounds_its_body_and_reports_its_depth() {
        let pose = CameraPose {
            target: Vec3::new(0.0, 1.0, 0.0),
            distance: 10.0,
            yaw_deg: 0.0,
            pitch_deg: 20.0,
            fov_deg: 40.0,
        };
        let view = pose.view();
        let view_proj = pose.projection(16.0 / 9.0, 0.1, 300.0).mul(&view);
        let figure = figure_box(&view, &view_proj, Vec3::ZERO, 1.8).unwrap();
        assert!(figure.rect[0] < 0.0 && figure.rect[2] > 0.0);
        assert!(figure.rect[1] < figure.rect[3]);
        assert!((figure.depth_m - 10.0).abs() < 1.5, "{}", figure.depth_m);
    }
}
