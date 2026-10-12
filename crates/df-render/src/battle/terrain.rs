//! Ground height and walkability taken from a scan's own splats.
//!
//! A scan has no ground mesh, so the height field is estimated from splat positions: in each
//! window the ground is the lowest *dense* layer of opaque splats (sparse floaters under the
//! surface do not count, grass and bushes above it do not either), holes are filled from their
//! neighbours and the result is lightly smoothed. Walkability then marks a cell blocked where the
//! ground jumps or where opaque splats stand between knee and head height (trunks, boulders,
//! walls). Both are authoring-time estimates: the numbers go into the scene file and the
//! artist reviews the result by eye. The runtime only samples the stored field.

use super::math::Vec3;

const MAX_SAMPLES: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq)]
pub enum TerrainError {
    Size,
    Step,
    Heights,
    NoGround,
}

/// Ground heights on a regular lattice of `cols * rows` points `step_m` apart, starting at
/// `origin` (x, z) in world metres.
#[derive(Clone, Debug, PartialEq)]
pub struct HeightField {
    origin: [f32; 2],
    step_m: f32,
    cols: u32,
    rows: u32,
    heights: Vec<f32>,
}

impl HeightField {
    pub fn new(
        origin: [f32; 2],
        step_m: f32,
        cols: u32,
        rows: u32,
        heights: Vec<f32>,
    ) -> Result<Self, TerrainError> {
        if cols < 2 || rows < 2 || (cols as usize).saturating_mul(rows as usize) > MAX_SAMPLES {
            return Err(TerrainError::Size);
        }
        if !step_m.is_finite() || !(0.05..=10.0).contains(&step_m) || origin.iter().any(|v| !v.is_finite()) {
            return Err(TerrainError::Step);
        }
        if heights.len() != cols as usize * rows as usize || heights.iter().any(|h| !h.is_finite()) {
            return Err(TerrainError::Heights);
        }
        Ok(Self {
            origin,
            step_m,
            cols,
            rows,
            heights,
        })
    }

    pub fn origin(&self) -> [f32; 2] {
        self.origin
    }

    pub fn step_m(&self) -> f32 {
        self.step_m
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

    fn at(&self, col: i64, row: i64) -> f32 {
        let col = col.clamp(0, i64::from(self.cols) - 1) as usize;
        let row = row.clamp(0, i64::from(self.rows) - 1) as usize;
        self.heights[row * self.cols as usize + col]
    }

    /// Bilinear height at a world point; points outside the lattice take the nearest edge value.
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let u = ((x - self.origin[0]) / self.step_m).clamp(0.0, (self.cols - 1) as f32);
        let v = ((z - self.origin[1]) / self.step_m).clamp(0.0, (self.rows - 1) as f32);
        let (col, row) = (u.floor() as i64, v.floor() as i64);
        let (fx, fz) = (u - col as f32, v - row as f32);
        let top = self.at(col, row) * (1.0 - fx) + self.at(col + 1, row) * fx;
        let bottom = self.at(col, row + 1) * (1.0 - fx) + self.at(col + 1, row + 1) * fx;
        top * (1.0 - fz) + bottom * fz
    }

    /// Largest height difference between lattice points inside the rectangle.
    pub fn relief_in(&self, min: [f32; 2], max: [f32; 2]) -> f32 {
        let (mut low, mut high) = (f32::MAX, f32::MIN);
        let steps = |from: f32, to: f32| {
            let count = ((to - from) / (self.step_m * 0.5)).ceil().max(1.0) as usize;
            (0..=count).map(move |i| from + (to - from) * i as f32 / count as f32)
        };
        for z in steps(min[1], max[1]) {
            for x in steps(min[0], max[0]) {
                let h = self.height_at(x, z);
                low = low.min(h);
                high = high.max(h);
            }
        }
        high - low
    }
}

/// A splat as the estimators see it: where it is and how opaque.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundPoint {
    pub position: Vec3,
    pub opacity: f32,
}

/// Estimation settings. Defaults suit a scan scaled to metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundSettings {
    /// Splats below this opacity are ignored.
    pub minimum_opacity: f32,
    /// Vertical extent of a "layer" of ground splats.
    pub band_m: f32,
    /// A layer needs at least this share of the splats in the window (and `minimum_count`).
    pub layer_share: f32,
    pub minimum_count: usize,
    /// Smoothing passes over the finished field.
    pub smoothing: u32,
}

impl Default for GroundSettings {
    fn default() -> Self {
        Self {
            minimum_opacity: 0.4,
            band_m: 0.2,
            layer_share: 0.08,
            minimum_count: 4,
            smoothing: 2,
        }
    }
}

/// Lowest dense layer of a window's heights, or `None` when no layer is dense enough.
fn lowest_dense_layer(heights: &mut [f32], settings: &GroundSettings) -> Option<f32> {
    if heights.len() < settings.minimum_count {
        return None;
    }
    heights.sort_by(f32::total_cmp);
    let need = ((heights.len() as f32 * settings.layer_share).ceil() as usize)
        .max(settings.minimum_count);
    (0..=heights.len().saturating_sub(need))
        .find(|&start| heights[start + need - 1] - heights[start] <= settings.band_m)
        .map(|start| heights[start + need / 2])
}

/// Estimates the ground height at every lattice point of the requested field from the splats.
/// Points with no dense layer are filled from their neighbours; a field with no ground at all is
/// an error rather than a flat guess.
pub fn estimate_height_field(
    points: &[GroundPoint],
    origin: [f32; 2],
    step_m: f32,
    cols: u32,
    rows: u32,
    settings: &GroundSettings,
) -> Result<HeightField, TerrainError> {
    let blank = HeightField::new(origin, step_m, cols, rows, vec![0.0; cols as usize * rows as usize])?;
    let (cols, rows) = (cols as usize, rows as usize);
    let mut windows: Vec<Vec<f32>> = vec![Vec::new(); cols * rows];
    for point in points.iter().filter(|p| p.opacity >= settings.minimum_opacity) {
        let u = (point.position.x - origin[0]) / step_m;
        let v = (point.position.z - origin[1]) / step_m;
        // Each splat counts for the lattice points within one step of it.
        for row in (v.floor() as i64)..=(v.floor() as i64 + 1) {
            for col in (u.floor() as i64)..=(u.floor() as i64 + 1) {
                if (0..cols as i64).contains(&col) && (0..rows as i64).contains(&row) {
                    windows[row as usize * cols + col as usize].push(point.position.y);
                }
            }
        }
    }
    let mut field: Vec<Option<f32>> = windows
        .iter_mut()
        .map(|window| lowest_dense_layer(window, settings))
        .collect();
    if field.iter().all(Option::is_none) {
        return Err(TerrainError::NoGround);
    }
    fill_holes(&mut field, cols, rows);
    let mut heights: Vec<f32> = field.into_iter().map(|h| h.unwrap_or(0.0)).collect();
    for _ in 0..settings.smoothing {
        heights = smoothed(&heights, cols, rows);
    }
    HeightField::new(blank.origin, step_m, cols as u32, rows as u32, heights)
}

fn fill_holes(field: &mut [Option<f32>], cols: usize, rows: usize) {
    loop {
        let snapshot = field.to_vec();
        let mut changed = false;
        for row in 0..rows {
            for col in 0..cols {
                if snapshot[row * cols + col].is_some() {
                    continue;
                }
                let mut sum = 0.0;
                let mut count = 0;
                for (dc, dr) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (c, r) = (col as i64 + dc, row as i64 + dr);
                    if (0..cols as i64).contains(&c)
                        && (0..rows as i64).contains(&r)
                        && let Some(h) = snapshot[r as usize * cols + c as usize]
                    {
                        sum += h;
                        count += 1;
                    }
                }
                if count > 0 {
                    field[row * cols + col] = Some(sum / count as f32);
                    changed = true;
                }
            }
        }
        if !changed {
            return;
        }
    }
}

fn smoothed(heights: &[f32], cols: usize, rows: usize) -> Vec<f32> {
    let mut out = heights.to_vec();
    for row in 0..rows {
        for col in 0..cols {
            let mut sum = 0.0;
            let mut weight = 0.0;
            for dr in -1i64..=1 {
                for dc in -1i64..=1 {
                    let (c, r) = (col as i64 + dc, row as i64 + dr);
                    if (0..cols as i64).contains(&c) && (0..rows as i64).contains(&r) {
                        let w = if dc == 0 && dr == 0 { 4.0 } else if dc == 0 || dr == 0 { 2.0 } else { 1.0 };
                        sum += heights[r as usize * cols + c as usize] * w;
                        weight += w;
                    }
                }
            }
            out[row * cols + col] = sum / weight;
        }
    }
    out
}

/// Settings for [`blocked_cells`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkSettings {
    /// A cell whose ground rises or falls more than this across it is blocked.
    pub maximum_relief_m: f32,
    /// Opaque splats above the ground by more than this (a knee) can block...
    pub knee_m: f32,
    /// ...up to this height (a head), so canopies and sky do not.
    pub head_m: f32,
    pub minimum_opacity: f32,
    /// This many blocking splats per square metre of cell blocks it.
    pub blockers_per_m2: f32,
}

impl Default for WalkSettings {
    fn default() -> Self {
        Self {
            maximum_relief_m: 0.6,
            knee_m: 0.5,
            head_m: 1.6,
            minimum_opacity: 0.5,
            blockers_per_m2: 40.0,
        }
    }
}

/// Cells (`[col, row]`) of a grid that a figure cannot stand in, judged from the ground height
/// and the splats standing above it.
pub fn blocked_cells(
    field: &HeightField,
    points: &[GroundPoint],
    origin: [f32; 2],
    cell_m: f32,
    cols: u32,
    rows: u32,
    settings: &WalkSettings,
) -> Vec<[i32; 2]> {
    let mut blockers = vec![0u32; cols as usize * rows as usize];
    for point in points.iter().filter(|p| p.opacity >= settings.minimum_opacity) {
        let col = ((point.position.x - origin[0]) / cell_m).floor();
        let row = ((point.position.z - origin[1]) / cell_m).floor();
        if col < 0.0 || row < 0.0 || col >= cols as f32 || row >= rows as f32 {
            continue;
        }
        let above = point.position.y - field.height_at(point.position.x, point.position.z);
        if above > settings.knee_m && above < settings.head_m {
            blockers[row as usize * cols as usize + col as usize] += 1;
        }
    }
    let threshold = settings.blockers_per_m2 * cell_m * cell_m;
    let mut blocked = Vec::new();
    for row in 0..rows as i32 {
        for col in 0..cols as i32 {
            let min = [origin[0] + col as f32 * cell_m, origin[1] + row as f32 * cell_m];
            let relief = field.relief_in(min, [min[0] + cell_m, min[1] + cell_m]);
            let count = blockers[row as usize * cols as usize + col as usize];
            if relief > settings.maximum_relief_m || count as f32 >= threshold {
                blocked.push([col, row]);
            }
        }
    }
    blocked
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u32);

    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as f32 / 16_777_216.0
        }
    }

    fn point(x: f32, y: f32, z: f32) -> GroundPoint {
        GroundPoint {
            position: Vec3::new(x, y, z),
            opacity: 0.9,
        }
    }

    /// A 10 m square of ground sloping 0.1 m per metre along x, with sparse floaters under it
    /// and grass and a boulder above it.
    fn scene() -> Vec<GroundPoint> {
        let mut random = Lcg(7);
        let mut points = Vec::new();
        for _ in 0..30_000 {
            let (x, z) = (random.next() * 10.0, random.next() * 10.0);
            points.push(point(x, x * 0.1 + random.next() * 0.04, z));
        }
        for _ in 0..40 {
            let (x, z) = (random.next() * 10.0, random.next() * 10.0);
            points.push(point(x, x * 0.1 - 1.0 - random.next(), z));
        }
        for _ in 0..4_000 {
            let (x, z) = (random.next() * 10.0, random.next() * 10.0);
            points.push(point(x, x * 0.1 + 0.1 + random.next() * 0.25, z));
        }
        // A boulder at x 6..7, z 2..3, a metre tall.
        for _ in 0..6_000 {
            let (x, z) = (6.0 + random.next(), 2.0 + random.next());
            points.push(point(x, x * 0.1 + random.next(), z));
        }
        points
    }

    #[test]
    fn the_ground_estimate_follows_a_slope_and_ignores_floaters_and_grass() {
        let field = estimate_height_field(&scene(), [0.0, 0.0], 0.5, 21, 21, &GroundSettings::default())
            .unwrap();
        for x in [1.0f32, 3.0, 5.0, 8.0] {
            for z in [1.0f32, 4.0, 8.0] {
                let truth = x * 0.1 + 0.02;
                let found = field.height_at(x, z);
                assert!((found - truth).abs() < 0.12, "at {x},{z}: {found} vs {truth}");
            }
        }
    }

    #[test]
    fn empty_windows_are_filled_from_their_neighbours_and_no_ground_is_an_error() {
        let points: Vec<_> = (0..2000)
            .map(|i| point((i % 40) as f32 * 0.1, 1.0, (i / 40) as f32 * 0.02))
            .collect();
        let field = estimate_height_field(&points, [0.0, 0.0], 0.5, 12, 12, &GroundSettings::default())
            .unwrap();
        assert!((field.height_at(5.0, 5.0) - 1.0).abs() < 0.05, "filled far corner");
        assert_eq!(
            estimate_height_field(&[], [0.0, 0.0], 0.5, 4, 4, &GroundSettings::default()),
            Err(TerrainError::NoGround)
        );
    }

    #[test]
    fn height_sampling_is_bilinear_and_clamps_outside_the_lattice() {
        let field = HeightField::new([0.0, 0.0], 1.0, 2, 2, vec![0.0, 1.0, 2.0, 3.0]).unwrap();
        assert_eq!(field.height_at(0.0, 0.0), 0.0);
        assert!((field.height_at(0.5, 0.5) - 1.5).abs() < 1e-6);
        assert!((field.height_at(1.0, 1.0) - 3.0).abs() < 1e-6);
        assert_eq!(field.height_at(-5.0, -5.0), 0.0);
        assert_eq!(field.height_at(9.0, 9.0), 3.0);
        assert_eq!(
            HeightField::new([0.0, 0.0], 1.0, 2, 2, vec![0.0; 3]),
            Err(TerrainError::Heights)
        );
        assert_eq!(HeightField::new([0.0, 0.0], 0.0, 2, 2, vec![0.0; 4]), Err(TerrainError::Step));
        assert_eq!(HeightField::new([0.0, 0.0], 1.0, 1, 2, vec![0.0; 2]), Err(TerrainError::Size));
    }

    #[test]
    fn a_boulder_blocks_its_cells_and_open_sloping_ground_stays_walkable() {
        let points = scene();
        let field = estimate_height_field(&points, [0.0, 0.0], 0.5, 21, 21, &GroundSettings::default())
            .unwrap();
        let blocked = blocked_cells(&field, &points, [0.0, 0.0], 1.0, 10, 10, &WalkSettings::default());
        assert!(blocked.contains(&[6, 2]), "boulder cell blocked: {blocked:?}");
        assert!(!blocked.contains(&[2, 7]), "open ground walkable");
        assert!(!blocked.contains(&[8, 5]));
    }

    #[test]
    fn a_step_in_the_ground_blocks_the_cell_it_crosses() {
        let mut heights = vec![0.0; 9 * 9];
        for row in 0..9 {
            for col in 5..9 {
                heights[row * 9 + col] = 2.0;
            }
        }
        let field = HeightField::new([0.0, 0.0], 1.0, 9, 9, heights).unwrap();
        let blocked = blocked_cells(&field, &[], [0.0, 0.0], 2.0, 4, 4, &WalkSettings::default());
        assert!(blocked.contains(&[2, 0]), "the cliff cell: {blocked:?}");
        assert!(!blocked.contains(&[0, 0]));
        assert!(!blocked.contains(&[3, 3]));
    }
}
