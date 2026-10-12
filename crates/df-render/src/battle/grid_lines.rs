//! The look of the battlefield grid lines and the data the grid pass reads.
//!
//! The grid is drawn as one analytic pass on the scene's flat play plane: straight lines with a
//! crisp bright core and a soft additive halo, coloured by the cells they border. Every number
//! that tunes the look lives here, in one place; the renderer copies them into the pass's data
//! block ([`grid_words`]) and the shader reads them from there. The pass is pure presentation:
//! cell codes come from where figures stand and which cells the server highlighted, never from
//! rules.

use super::{
    grid::{Cell, Grid},
    model::{HighlightKind, Side},
};

/// Width of the bright core, in metres. Never drawn thinner than [`CORE_MIN_PX`] on screen; a line
/// thinner than that fades instead of shimmering.
pub const LINE_WIDTH_M: f32 = 0.012;
/// Narrowest the core is drawn, in pixels, so it stays anti-aliased at distance.
pub const CORE_MIN_PX: f32 = 1.0;
/// Opacity of the core at full strength.
pub const CORE_INTENSITY: f32 = 0.55;
/// How much of the line colour the core is whitened by: a bright core reads as a lit filament.
pub const CORE_WHITENING: f32 = 0.2;
/// Width of the soft halo around a line, in metres.
pub const HALO_WIDTH_M: f32 = 0.05;
/// Narrowest the halo is drawn, in pixels; below this world width it fades out with distance.
pub const HALO_MIN_PX: f32 = 2.5;
/// Strength of the additive halo.
pub const HALO_INTENSITY: f32 = 0.0;
/// Opacity of the dark underlay beside the line, which keeps the glow legible on bright ground
/// (daylight grass) where an additive halo alone would wash out.
pub const UNDERLAY_ALPHA: f32 = 0.1;
/// Opacity of a line seen through scan geometry that is in front of it: the line dims, it does
/// not break up.
pub const SEEN_THROUGH_ALPHA: f32 = 0.32;
/// How far in front of a line the scan has to be before the line counts as hidden, in metres
/// along the line of sight: grass and rubble at the ground do not hide it, a hut or a wall does.
pub const OCCLUSION_MARGIN_M: f32 = 2.0;
/// Brightness multiplier for the outline of a reachable or targeted region (the lines inside it
/// keep the base strength, tinted): a little brighter than the grid, still a thin line.
pub const HIGHLIGHT_BOOST: f32 = 1.6;
/// A cell narrower than this many pixels (the lines crowd together) fades the lines out.
pub const CELL_MIN_PX: f32 = 8.0;
/// Distance from the grid centre where the lines start to dim, and where they stop, in metres.
pub const CENTRE_FADE_START_M: f32 = 3.5;
pub const CENTRE_FADE_END_M: f32 = 10.0;
/// Share of strength kept at and beyond [`CENTRE_FADE_END_M`].
pub const CENTRE_FADE_FLOOR: f32 = 0.22;
/// How far in from the edge of the play area the lines are dimmed, in cells, and the share of
/// strength the outermost line keeps. Highlighted lines keep most of their strength regardless.
pub const EDGE_FADE_CELLS: f32 = 1.8;
pub const EDGE_FADE_FLOOR: f32 = 0.3;
/// How far the drawn quad extends beyond the grid, so the halo of the outer lines is not cut.
pub const QUAD_MARGIN_M: f32 = 0.6;
/// How far above the play plane the lines sit, in metres: enough to clear the ground surface.
pub const PLANE_LIFT_M: f32 = 0.012;

/// Strength of the lines around a blocked cell, as a share of a walkable one: the lattice stays
/// continuous, the unusable cells just read as dimmer.
pub const BLOCKED_STRENGTH: f32 = 0.3;

/// The warm gold of the neutral grid.
pub const GOLD: [f32; 3] = [0.96, 0.78, 0.42];
/// The cool blue of hero cells.
pub const HERO_BLUE: [f32; 3] = [0.5, 0.86, 1.0];
/// The red of enemy cells.
pub const ENEMY_RED: [f32; 3] = [1.0, 0.34, 0.28];

/// Words before the per-cell codes in the pass's data block.
pub const HEADER_WORDS: usize = 36;

/// A blocked cell inside the grid: faint lines.
const CODE_BLOCKED: f32 = 0.5;
const CODE_NEUTRAL: f32 = 1.0;
const CODE_ENEMY: f32 = 2.0;
const CODE_HERO: f32 = 3.0;
const CODE_REACHABLE: f32 = 4.0;
const CODE_TARGET: f32 = 5.0;

/// The data block for the grid pass: [`HEADER_WORDS`] words of placement and look, then one code
/// per cell, row by row. A line takes the highest code of the two cells it separates, in the order
/// blocked, neutral, enemy, hero, reachable, target.
pub fn grid_words(
    grid: &Grid,
    standing: &[(Cell, Side)],
    highlights: &[(HighlightKind, &[Cell])],
) -> Vec<f32> {
    let origin = grid.origin();
    let mut words = vec![0.0; HEADER_WORDS];
    words[0] = origin[0];
    words[1] = origin[1];
    words[2] = grid.cell_m();
    words[3] = grid.cols() as f32;
    words[4] = grid.rows() as f32;
    words[5] = grid.play_plane_y() + PLANE_LIFT_M;
    words[6] = LINE_WIDTH_M;
    words[7] = CORE_MIN_PX;
    words[8] = CORE_INTENSITY;
    words[9] = HALO_WIDTH_M;
    words[10] = HALO_MIN_PX;
    words[11] = HALO_INTENSITY;
    words[12] = UNDERLAY_ALPHA;
    words[13] = SEEN_THROUGH_ALPHA;
    words[14] = HIGHLIGHT_BOOST;
    words[15] = CELL_MIN_PX;
    words[16] = CENTRE_FADE_START_M;
    words[17] = CENTRE_FADE_END_M;
    words[18] = CENTRE_FADE_FLOOR;
    words[19] = QUAD_MARGIN_M;
    words[20..23].copy_from_slice(&GOLD);
    words[24..27].copy_from_slice(&HERO_BLUE);
    words[28..31].copy_from_slice(&ENEMY_RED);
    words[23] = CORE_WHITENING;
    words[31] = BLOCKED_STRENGTH;
    words[32] = EDGE_FADE_CELLS;
    words[33] = EDGE_FADE_FLOOR;
    for row in 0..grid.rows() as i32 {
        for col in 0..grid.cols() as i32 {
            let cell = Cell::new(col, row);
            words.push(if grid.is_walkable(cell) {
                CODE_NEUTRAL
            } else {
                CODE_BLOCKED
            });
        }
    }
    let mut raise = |cell: Cell, code: f32| {
        if grid.is_walkable(cell) {
            let index = HEADER_WORDS + (cell.row as u32 * grid.cols() + cell.col as u32) as usize;
            words[index] = words[index].max(code);
        }
    };
    for (cell, side) in standing {
        raise(
            *cell,
            match side {
                Side::Hero => CODE_HERO,
                Side::Enemy => CODE_ENEMY,
            },
        );
    }
    for (kind, cells) in highlights {
        let code = match kind {
            HighlightKind::Move => CODE_REACHABLE,
            HighlightKind::Attack => CODE_TARGET,
        };
        for cell in *cells {
            raise(*cell, code);
        }
    }
    words
}

/// How far along the line of sight (metres from the eye) a figure standing on the play plane is
/// tested for depth: its own distance, or where the line of sight crosses `margin_m` above the
/// plane if that is nearer. Ground up to the margin never hides the figure; taller things in front
/// of it still do. This is the CPU twin of `standing_depth` in the shaders, kept for testing.
pub fn standing_distance_m(
    eye_height_m: f32,
    plane_m: f32,
    margin_m: f32,
    direction_y: f32,
    own_distance_m: f32,
) -> f32 {
    if direction_y < -0.0001 {
        own_distance_m.min((eye_height_m - (plane_m + margin_m)) / -direction_y)
    } else {
        own_distance_m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        Grid::open_except([-3.0, -2.0], 1.5, 4, 3, &[Cell::new(1, 1)]).unwrap()
    }

    fn code(words: &[f32], cols: usize, cell: Cell) -> f32 {
        words[HEADER_WORDS + cell.row as usize * cols + cell.col as usize]
    }

    #[test]
    fn the_block_has_a_fixed_header_then_one_code_per_cell() {
        let words = grid_words(&grid(), &[], &[]);
        assert_eq!(words.len(), HEADER_WORDS + 12);
        assert_eq!(&words[0..5], &[-3.0, -2.0, 1.5, 4.0, 3.0]);
        assert!(
            (words[5] - PLANE_LIFT_M).abs() < 1e-6,
            "plane height plus lift"
        );
        assert_eq!(&words[20..23], &GOLD);
        assert_eq!(&words[24..27], &HERO_BLUE);
        assert_eq!(&words[28..31], &ENEMY_RED);
    }

    #[test]
    fn blocked_cells_keep_faint_lines_and_walkable_cells_are_neutral_gold() {
        let words = grid_words(&grid(), &[], &[]);
        assert_eq!(code(&words, 4, Cell::new(1, 1)), CODE_BLOCKED);
        assert_eq!(code(&words, 4, Cell::new(0, 0)), CODE_NEUTRAL);
        assert_eq!(code(&words, 4, Cell::new(3, 2)), CODE_NEUTRAL);
    }

    #[test]
    fn figures_colour_their_own_cell_by_side() {
        let standing = [
            (Cell::new(0, 0), Side::Hero),
            (Cell::new(3, 2), Side::Enemy),
        ];
        let words = grid_words(&grid(), &standing, &[]);
        assert_eq!(code(&words, 4, Cell::new(0, 0)), CODE_HERO);
        assert_eq!(code(&words, 4, Cell::new(3, 2)), CODE_ENEMY);
        assert_eq!(code(&words, 4, Cell::new(2, 0)), CODE_NEUTRAL);
    }

    #[test]
    fn highlights_outrank_figures_and_never_light_blocked_cells() {
        let standing = [(Cell::new(0, 0), Side::Hero)];
        let cells = [Cell::new(0, 0), Cell::new(1, 1), Cell::new(9, 9)];
        let moved = grid_words(&grid(), &standing, &[(HighlightKind::Move, &cells)]);
        assert_eq!(code(&moved, 4, Cell::new(0, 0)), CODE_REACHABLE);
        assert_eq!(
            code(&moved, 4, Cell::new(1, 1)),
            CODE_BLOCKED,
            "blocked stays faint"
        );
        let attacked = grid_words(&grid(), &standing, &[(HighlightKind::Attack, &cells)]);
        assert_eq!(code(&attacked, 4, Cell::new(0, 0)), CODE_TARGET);
    }

    #[test]
    fn the_look_constants_are_sane() {
        const { assert!(LINE_WIDTH_M < HALO_WIDTH_M) };
        const { assert!(CORE_MIN_PX >= 1.0 && HALO_MIN_PX > CORE_MIN_PX) };
        for value in [
            CORE_INTENSITY,
            CORE_WHITENING,
            BLOCKED_STRENGTH,
            HALO_INTENSITY,
            UNDERLAY_ALPHA,
            SEEN_THROUGH_ALPHA,
            CENTRE_FADE_FLOOR,
        ] {
            assert!((0.0..=1.0).contains(&value));
        }
        const { assert!(HIGHLIGHT_BOOST > 1.0) };
        const { assert!(CENTRE_FADE_START_M < CENTRE_FADE_END_M) };
        const { assert!(QUAD_MARGIN_M > HALO_WIDTH_M * 2.0) };
    }

    #[test]
    fn standing_figures_are_pulled_forward_at_the_feet_only() {
        // Camera 10 m up looking down 30 degrees at a figure 20 m away along the line of sight.
        let direction_y = -(30.0_f32.to_radians()).sin();
        let eye = 10.0;
        let own = eye / -direction_y;
        // Feet (height 0): pulled toward the camera by margin / sin(30) = twice the margin.
        let feet = standing_distance_m(eye, 0.0, 0.6, direction_y, own);
        assert!((own - feet - 1.2).abs() < 1e-4, "{own} {feet}");
        // A pixel on the figure 0.6 m up (the margin) or higher is not moved.
        let head_own = (eye - 1.5) / -direction_y;
        assert_eq!(
            standing_distance_m(eye, 0.0, 0.6, direction_y, head_own),
            head_own
        );
        // Without a margin (painted grounds) nothing moves.
        assert_eq!(standing_distance_m(eye, 0.0, 0.0, direction_y, own), own);
        // Looking up or level: unchanged.
        assert_eq!(standing_distance_m(eye, 0.0, 0.6, 0.2, own), own);
    }

    #[test]
    fn a_wall_nearer_than_the_margin_crossing_still_hides_the_figure() {
        let direction_y = -(30.0_f32.to_radians()).sin();
        let eye = 10.0;
        let own = eye / -direction_y;
        let tested = standing_distance_m(eye, 0.0, 0.6, direction_y, own);
        // Ground at 0.3 m above the plane at this pixel is further than the margin crossing.
        let ground = (eye - 0.3) / -direction_y;
        assert!(tested <= ground, "turf does not hide the feet");
        // A palisade face crossing the line of sight 1.4 m up is nearer: it hides the figure.
        let palisade = (eye - 1.4) / -direction_y;
        assert!(palisade < tested, "a tall occluder still hides the feet");
    }
}
