//! A fight grid read from a scanned world: which squares of a room are open floor, which are
//! blocked, and which have no floor at all.
//!
//! The collider mesh (see [`ColliderMesh`]) is the primary source. Every triangle is sampled
//! over its surface, each sample carrying its share of the triangle's area, and the samples are
//! gathered per square:
//!
//! - upward-facing surface within the floor tolerance of y = 0 is floor;
//! - any other surface between the hero band's lower and upper heights (a wall, a counter, a
//!   post, a table top, a platform too high to step onto) is something a figure cannot stand
//!   in, and blocks the square once it adds up to a share of a wall across it;
//! - a square that is not blocked and has too little floor is [`WorldCell::NoFloor`] (a pit, a
//!   doorway into the void, the edge of the capture), otherwise it is [`WorldCell::Open`].
//!
//! Splat centres on the other hand are noisy indoors, so the voxel probe over the splats is only
//! a second opinion: where it finds contact in the hero band the square is blocked, whatever the
//! mesh said. The probe can add blocked squares and never remove one.
//!
//! The grid is staging data for the narrative engine and the tactical view, not rules authority:
//! the figures walk on squares the server owns. Everything here is pure and deterministic: the
//! same inputs give the same grid, bit for bit.

use super::{
    collider::{ColliderError, VoxelBuilder},
    collider_mesh::ColliderMesh,
    math::Vec3,
    splat::{RawSplat, SplatCloud, SplatTransform},
};

/// A D&D square: five feet.
pub const DEFAULT_CELL_M: f32 = 1.524;
/// The most squares along either side of a world grid.
pub const MAX_WORLD_GRID_SIDE: u32 = 256;

const MAX_SAMPLES: u64 = 200_000_000;
const SAMPLE_SPACING_M: f32 = 0.05;
const MAX_SUBDIVISIONS: u32 = 64;
const PROBE_CHUNK: usize = 100_000;
const PROBE_MARGIN_M: f32 = 0.5;
const PROBE_SPACING_M: f32 = 0.5;
const PROBE_MAX_SUBDIVISIONS: u32 = 8;
const PROBE_BODY_RADIUS_M: f32 = 0.35;
const PROBE_FLOOR_ROOM_M: f32 = 0.2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorldCell {
    /// Floor a figure can stand on, with nothing in the hero band.
    Open,
    /// Something solid in the hero band: a wall, furniture, a fence, a platform.
    Blocked,
    /// Neither floor nor an obstacle: a pit, a gap in the capture, outside the room.
    NoFloor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldGridSettings {
    /// Square size in metres, 0.1 to 20.
    pub cell_m: f32,
    /// Heights above the floor, `[lowest, highest]`, between which a surface blocks a figure:
    /// below the lowest it can be stepped over, above the highest it is overhead.
    pub hero_band_m: [f32; 2],
    /// How far from y = 0 a surface still counts as the floor.
    pub floor_tolerance_m: f32,
    /// The smallest vertical component of a surface normal, 0 to 1, that counts as facing up.
    pub floor_normal_y_min: f32,
    /// The share of a square, 0 to 1, that floor must cover for the square to have a floor.
    pub floor_coverage: f32,
    /// How much surface in the hero band blocks a square, as a share (0 to 1) of a wall running
    /// straight across it: the square's side times the band's height. Scaling with the square
    /// keeps a corner of wall from blocking a whole five-foot square; 0.2 of a half-metre
    /// square is the 0.15 m2 measured on the first Marble room.
    pub blocking_wall_share: f32,
    /// Voxel size of the optional splat probe, 0.025 to 0.25 m.
    pub probe_voxel_m: f32,
    /// Where the squares start and how far they reach, in world x, z: square (0, 0) has its
    /// corner at `min` exactly, there are as many squares as it takes to reach `max`, and mesh
    /// outside the footprint is left out. `None` covers the whole mesh on a lattice of
    /// multiples of the square size. Use it to keep to one room of a world that includes its
    /// surroundings.
    pub footprint: Option<WorldBounds>,
}

impl Default for WorldGridSettings {
    /// A five-foot square, a hero who clears 0.3 m and stands 1.7 m, and the thresholds measured
    /// on the first Marble room.
    fn default() -> Self {
        Self {
            cell_m: DEFAULT_CELL_M,
            hero_band_m: [0.3, 1.7],
            floor_tolerance_m: 0.2,
            floor_normal_y_min: 0.8,
            floor_coverage: 0.08,
            blocking_wall_share: 0.2,
            probe_voxel_m: 0.1,
            footprint: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorldGridError {
    CellSize,
    HeroBand,
    FloorTolerance,
    FloorNormal,
    FloorCoverage,
    BlockingWallShare,
    ProbeVoxel,
    /// A footprint that is not finite or has no area.
    Footprint,
    /// The world transform has a non-positive or non-finite scale.
    Transform,
    /// A vertex that is not finite once placed in the world.
    NonFiniteGeometry,
    /// More than 256 squares along a side at this square size.
    GridTooLarge,
    /// The mesh is larger than the surface sampling budget.
    TooManySamples,
    /// The voxel probe could not be built or queried.
    Probe(ColliderError),
}

impl WorldGridSettings {
    fn validate(&self) -> Result<(), WorldGridError> {
        let [low, high] = self.hero_band_m;
        if !(0.1..=20.0).contains(&self.cell_m) {
            return Err(WorldGridError::CellSize);
        }
        if !(0.0..=10.0).contains(&low) || !(0.0..=10.0).contains(&high) || low >= high {
            return Err(WorldGridError::HeroBand);
        }
        if !(0.0..=1.0).contains(&self.floor_tolerance_m) {
            return Err(WorldGridError::FloorTolerance);
        }
        if !(0.0..=1.0).contains(&self.floor_normal_y_min) || self.floor_normal_y_min == 0.0 {
            return Err(WorldGridError::FloorNormal);
        }
        if !(0.0..=1.0).contains(&self.floor_coverage) || self.floor_coverage == 0.0 {
            return Err(WorldGridError::FloorCoverage);
        }
        if !(0.0..=1.0).contains(&self.blocking_wall_share) || self.blocking_wall_share == 0.0 {
            return Err(WorldGridError::BlockingWallShare);
        }
        if !(0.025..=0.25).contains(&self.probe_voxel_m) {
            return Err(WorldGridError::ProbeVoxel);
        }
        if let Some(WorldBounds { min, max }) = self.footprint
            && !(min.into_iter().chain(max).all(f32::is_finite)
                && min[0] < max[0]
                && min[1] < max[1])
        {
            return Err(WorldGridError::Footprint);
        }
        Ok(())
    }
}

/// How many squares of each kind a grid has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellCounts {
    pub open: u32,
    pub blocked: u32,
    pub no_floor: u32,
}

/// The biggest group of open squares that touch edge to edge (a figure cannot squeeze past the
/// corner of two blocked squares, so diagonals do not connect).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpenArea {
    pub squares: u32,
    /// The same area in square metres (`squares` times the square's area), which compares across
    /// square sizes: the squares you get depend on where the walls fall against the lattice.
    pub area_m2: f32,
    /// The width and depth of the box that just holds the group, in squares; a corridor of 20
    /// squares and a hall of 20 squares differ here.
    pub extent_squares: [u32; 2],
}

/// Where the splat probe and the mesh agreed. Only blocked squares are compared.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CrossCheck {
    pub blocked_by_both: u32,
    /// Squares the mesh left open (or without floor) that the probe blocked; these are blocked
    /// in the grid.
    pub blocked_by_probe_only: u32,
    pub blocked_by_mesh_only: u32,
}

/// The ground footprint of a grid in world metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldBounds {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldGrid {
    origin: [f32; 2],
    cell_m: f32,
    cols: u32,
    rows: u32,
    cells: Vec<WorldCell>,
    mesh_bounds: [Vec3; 2],
    cross_check: Option<CrossCheck>,
}

impl WorldGrid {
    /// World x, z of the corner of square (0, 0). Columns grow along +x and rows along +z, as in
    /// the battlefield [`Grid`](super::grid::Grid).
    pub fn origin(&self) -> [f32; 2] {
        self.origin
    }

    pub fn cell_m(&self) -> f32 {
        self.cell_m
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    pub fn cell(&self, col: u32, row: u32) -> Option<WorldCell> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        self.cells
            .get(row as usize * self.cols as usize + col as usize)
            .copied()
    }

    /// World x, z of the middle of a square.
    pub fn cell_centre(&self, col: u32, row: u32) -> Option<[f32; 2]> {
        (col < self.cols && row < self.rows).then(|| {
            [
                self.origin[0] + (col as f32 + 0.5) * self.cell_m,
                self.origin[1] + (row as f32 + 0.5) * self.cell_m,
            ]
        })
    }

    /// The footprint the squares cover.
    pub fn bounds(&self) -> WorldBounds {
        WorldBounds {
            min: self.origin,
            max: [
                self.origin[0] + self.cols as f32 * self.cell_m,
                self.origin[1] + self.rows as f32 * self.cell_m,
            ],
        }
    }

    /// The lowest and highest corners of the mesh itself, in world metres.
    pub fn mesh_bounds(&self) -> [Vec3; 2] {
        self.mesh_bounds
    }

    pub fn counts(&self) -> CellCounts {
        let count = |wanted| self.cells.iter().filter(|cell| **cell == wanted).count() as u32;
        CellCounts {
            open: count(WorldCell::Open),
            blocked: count(WorldCell::Blocked),
            no_floor: count(WorldCell::NoFloor),
        }
    }

    /// Row-major indices (`row * cols + col`) of the open squares: the walkable list that
    /// [`Grid::new`](super::grid::Grid::new) takes.
    pub fn walkable_indices(&self) -> Vec<u32> {
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| **cell == WorldCell::Open)
            .map(|(index, _)| index as u32)
            .collect()
    }

    /// The largest connected open area; `None` when no square is open.
    pub fn largest_open_area(&self) -> Option<OpenArea> {
        let (cols, rows) = (self.cols as usize, self.rows as usize);
        let mut seen = vec![false; self.cells.len()];
        let mut best: Option<OpenArea> = None;
        let mut stack: Vec<usize> = Vec::new();
        for start in 0..self.cells.len() {
            if seen[start] || self.cells[start] != WorldCell::Open {
                continue;
            }
            seen[start] = true;
            stack.push(start);
            let (mut squares, mut min, mut max) = (0u32, [usize::MAX; 2], [0usize; 2]);
            while let Some(at) = stack.pop() {
                let (col, row) = (at % cols, at / cols);
                squares += 1;
                min = [min[0].min(col), min[1].min(row)];
                max = [max[0].max(col), max[1].max(row)];
                let neighbours = [
                    (col > 0).then(|| at - 1),
                    (col + 1 < cols).then(|| at + 1),
                    (row > 0).then(|| at - cols),
                    (row + 1 < rows).then(|| at + cols),
                ];
                for next in neighbours.into_iter().flatten() {
                    if !seen[next] && self.cells[next] == WorldCell::Open {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
            // Ties keep the group found first in row-major order, so the result is stable.
            if best.is_none_or(|area| squares > area.squares) {
                best = Some(OpenArea {
                    squares,
                    area_m2: squares as f32 * self.cell_m * self.cell_m,
                    extent_squares: [(max[0] - min[0] + 1) as u32, (max[1] - min[1] + 1) as u32],
                });
            }
        }
        best
    }

    /// How the splat probe compared with the mesh; `None` when no splats were given.
    pub fn cross_check(&self) -> Option<CrossCheck> {
        self.cross_check
    }
}

/// Reads the walkable squares of a scanned world.
///
/// `mesh` is the collider in the scan's own frame and `transform` places it in the world (for
/// Marble, [`marble_world_transform`](super::ply::marble_world_transform)); the floor is taken
/// to be at world y = 0. `probe_splats`, the scan's splats in the same frame, are optional: when
/// given, a voxel probe over them may block further squares (see the module notes).
///
/// Squares are aligned to multiples of the square size in world x and z and cover the mesh.
pub fn world_grid(
    mesh: &ColliderMesh,
    transform: &SplatTransform,
    settings: &WorldGridSettings,
    probe_splats: Option<&[RawSplat]>,
) -> Result<WorldGrid, WorldGridError> {
    settings.validate()?;
    if !transform.scale.is_finite() || transform.scale <= 0.0 {
        return Err(WorldGridError::Transform);
    }
    let world: Vec<Vec3> = mesh
        .vertices()
        .iter()
        .map(|vertex| transform.to_world(*vertex))
        .collect();
    if world.iter().any(|vertex| !vertex.is_finite()) {
        return Err(WorldGridError::NonFiniteGeometry);
    }

    // The mesh guarantees every triangle index is below the vertex count.
    let corners = |triangle: &[u32; 3]| triangle.map(|index| world[index as usize]);
    let mut low = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
    let mut high = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
    for triangle in mesh.triangles() {
        for corner in corners(triangle) {
            low = Vec3::new(
                low.x.min(corner.x),
                low.y.min(corner.y),
                low.z.min(corner.z),
            );
            high = Vec3::new(
                high.x.max(corner.x),
                high.y.max(corner.y),
                high.z.max(corner.z),
            );
        }
    }

    let cell = settings.cell_m;
    let (origin, reach) = match settings.footprint {
        Some(footprint) => (footprint.min, footprint.max),
        None => (
            [(low.x / cell).floor() * cell, (low.z / cell).floor() * cell],
            [high.x, high.z],
        ),
    };
    let side = |from: f32, to: f32| ((to - from) / cell).ceil().max(1.0);
    let (cols, rows) = (side(origin[0], reach[0]), side(origin[1], reach[1]));
    if cols > MAX_WORLD_GRID_SIDE as f32 || rows > MAX_WORLD_GRID_SIDE as f32 {
        return Err(WorldGridError::GridTooLarge);
    }
    let (cols, rows) = (cols as u32, rows as u32);

    let (floor_area, block_area) =
        gather_surface_areas(mesh, &corners, settings, origin, cols, rows)?;
    let cell_area = f64::from(cell) * f64::from(cell);
    let [band_low, band_high] = settings.hero_band_m;
    let blocking_area =
        f64::from(settings.blocking_wall_share) * f64::from(cell) * f64::from(band_high - band_low);
    let mut blocked_by_mesh: Vec<bool> = Vec::with_capacity(floor_area.len());
    let mut cells: Vec<WorldCell> = Vec::with_capacity(floor_area.len());
    for (floor, block) in floor_area.iter().zip(&block_area) {
        let blocked = *block >= blocking_area;
        blocked_by_mesh.push(blocked);
        cells.push(if blocked {
            WorldCell::Blocked
        } else if *floor < f64::from(settings.floor_coverage) * cell_area {
            WorldCell::NoFloor
        } else {
            WorldCell::Open
        });
    }

    let mut cross_check = None;
    if let Some(splats) = probe_splats {
        let blocked_by_probe =
            probe_blocked_squares(splats, transform, settings, origin, cols, rows)?;
        let mut check = CrossCheck {
            blocked_by_both: 0,
            blocked_by_probe_only: 0,
            blocked_by_mesh_only: 0,
        };
        for ((cell, by_mesh), by_probe) in cells
            .iter_mut()
            .zip(&blocked_by_mesh)
            .zip(&blocked_by_probe)
        {
            match (*by_mesh, *by_probe) {
                (true, true) => check.blocked_by_both += 1,
                (true, false) => check.blocked_by_mesh_only += 1,
                (false, true) => check.blocked_by_probe_only += 1,
                (false, false) => {}
            }
            if *by_probe {
                *cell = WorldCell::Blocked;
            }
        }
        cross_check = Some(check);
    }

    Ok(WorldGrid {
        origin,
        cell_m: cell,
        cols,
        rows,
        cells,
        mesh_bounds: [low, high],
        cross_check,
    })
}

/// Per square: the area of floor-like surface, and the area of surface in the hero band.
fn gather_surface_areas(
    mesh: &ColliderMesh,
    corners: &dyn Fn(&[u32; 3]) -> [Vec3; 3],
    settings: &WorldGridSettings,
    origin: [f32; 2],
    cols: u32,
    rows: u32,
) -> Result<(Vec<f64>, Vec<f64>), WorldGridError> {
    let squares = cols as usize * rows as usize;
    let mut floor_area = vec![0.0f64; squares];
    let mut block_area = vec![0.0f64; squares];
    let [band_low, band_high] = settings.hero_band_m;
    let mut samples = 0u64;
    for triangle in mesh.triangles() {
        let [a, b, c] = corners(triangle);
        let normal = (b - a).cross(c - a);
        let double_area = normal.length();
        if !double_area.is_finite() {
            return Err(WorldGridError::NonFiniteGeometry);
        }
        let area = f64::from(double_area) * 0.5;
        // A sliver with no area has no surface to sample.
        if area <= 1e-9 {
            continue;
        }
        let facing_up = (normal.y / double_area).abs() >= settings.floor_normal_y_min;
        let longest = (b - a).length().max((c - b).length()).max((a - c).length());
        let n = ((longest / SAMPLE_SPACING_M).ceil() as u32).clamp(1, MAX_SUBDIVISIONS);
        samples += u64::from(n) * u64::from(n);
        if samples > MAX_SAMPLES {
            return Err(WorldGridError::TooManySamples);
        }
        let weight = area / f64::from(n * n);
        let step = 1.0 / n as f32;
        let mut deposit = |u: f32, v: f32| {
            let point = a + (b - a) * u + (c - a) * v;
            let col = ((point.x - origin[0]) / settings.cell_m).floor() as i64;
            let row = ((point.z - origin[1]) / settings.cell_m).floor() as i64;
            // Without a footprint every sample is inside, up to rounding on the far edge, which
            // belongs to the last square. With one, mesh outside it is not part of the grid.
            let (col, row) = if settings.footprint.is_some() {
                if !(0..i64::from(cols)).contains(&col) || !(0..i64::from(rows)).contains(&row) {
                    return;
                }
                (col, row)
            } else {
                (
                    col.clamp(0, i64::from(cols) - 1),
                    row.clamp(0, i64::from(rows) - 1),
                )
            };
            let at = row as usize * cols as usize + col as usize;
            // `at` is below `cols * rows` by the checks above.
            if facing_up && point.y.abs() <= settings.floor_tolerance_m {
                floor_area[at] += weight;
            } else if point.y > band_low && point.y < band_high {
                block_area[at] += weight;
            }
        };
        // The n x n sub-triangles of the lattice, each sampled at its centroid.
        for i in 0..n {
            for j in 0..(n - i) {
                let (fi, fj) = (i as f32, j as f32);
                deposit((fi + 1.0 / 3.0) * step, (fj + 1.0 / 3.0) * step);
                if j + 1 < n - i {
                    deposit((fi + 2.0 / 3.0) * step, (fj + 2.0 / 3.0) * step);
                }
            }
        }
    }
    Ok((floor_area, block_area))
}

/// For each square, whether the splats put solid matter in the hero band there. Each square is
/// tested with a lattice of upright capsules about as wide as a person, so a square wider than
/// one body is covered across, not only at its middle.
fn probe_blocked_squares(
    splats: &[RawSplat],
    transform: &SplatTransform,
    settings: &WorldGridSettings,
    origin: [f32; 2],
    cols: u32,
    rows: u32,
) -> Result<Vec<bool>, WorldGridError> {
    let [band_low, band_high] = settings.hero_band_m;
    let floor_y = -(settings.floor_tolerance_m + PROBE_FLOOR_ROOM_M);
    let top_y = band_high + PROBE_FLOOR_ROOM_M;
    let voxel = settings.probe_voxel_m;
    let dims = [
        ((cols as f32 * settings.cell_m + 2.0 * PROBE_MARGIN_M) / voxel).ceil() as u32,
        ((top_y - floor_y) / voxel).ceil() as u32,
        ((rows as f32 * settings.cell_m + 2.0 * PROBE_MARGIN_M) / voxel).ceil() as u32,
    ];
    let volume_origin = Vec3::new(
        origin[0] - PROBE_MARGIN_M,
        floor_y,
        origin[1] - PROBE_MARGIN_M,
    );
    let mut builder =
        VoxelBuilder::new(volume_origin, voxel, dims).map_err(WorldGridError::Probe)?;
    for chunk in splats.chunks(PROBE_CHUNK) {
        let (cloud, dropped) = SplatCloud::bake(chunk, *transform);
        if dropped > 0 {
            return Err(WorldGridError::Probe(ColliderError::Splat));
        }
        builder.add(cloud.splats()).map_err(WorldGridError::Probe)?;
    }
    let collider = builder.finish().map_err(WorldGridError::Probe)?;

    let per_side =
        ((settings.cell_m / PROBE_SPACING_M).ceil() as u32).clamp(1, PROBE_MAX_SUBDIVISIONS);
    let sub = settings.cell_m / per_side as f32;
    let radius = (sub * 0.5).min(PROBE_BODY_RADIUS_M);
    // The capsule's spine runs between the band's ends less the radius, so the capsule fills the
    // band exactly; a band narrower than a body is a single point.
    let (spine_low, spine_high) = if band_low + radius < band_high - radius {
        (band_low + radius, band_high - radius)
    } else {
        let middle = 0.5 * (band_low + band_high);
        (middle, middle)
    };
    let mut blocked = Vec::with_capacity(cols as usize * rows as usize);
    for row in 0..rows {
        for col in 0..cols {
            let (x0, z0) = (
                origin[0] + col as f32 * settings.cell_m,
                origin[1] + row as f32 * settings.cell_m,
            );
            let mut touched = false;
            'lattice: for i in 0..per_side {
                for j in 0..per_side {
                    let x = x0 + (i as f32 + 0.5) * sub;
                    let z = z0 + (j as f32 + 0.5) * sub;
                    if collider
                        .touches_capsule(
                            Vec3::new(x, spine_low, z),
                            Vec3::new(x, spine_high, z),
                            radius,
                        )
                        .map_err(WorldGridError::Probe)?
                    {
                        touched = true;
                        break 'lattice;
                    }
                }
            }
            blocked.push(touched);
        }
    }
    Ok(blocked)
}

#[cfg(test)]
mod tests {
    use super::super::{
        collider_mesh::test_glb, grid::Grid, math::Quat, ply::marble_world_transform,
    };
    use super::*;

    /// Square size 1 m keeps the fixtures countable by eye.
    fn unit() -> WorldGridSettings {
        WorldGridSettings {
            cell_m: 1.0,
            ..WorldGridSettings::default()
        }
    }

    #[derive(Default)]
    struct Soup {
        vertices: Vec<Vec3>,
        triangles: Vec<[u32; 3]>,
    }

    impl Soup {
        fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3) {
            let base = self.vertices.len() as u32;
            self.vertices.extend([a, b, c, d]);
            self.triangles
                .extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
        }

        /// A flat patch at height `y` over the squares `[x0, x1) x [z0, z1)`.
        fn floor(&mut self, x: [f32; 2], z: [f32; 2], y: f32) {
            self.quad(
                Vec3::new(x[0], y, z[0]),
                Vec3::new(x[1], y, z[0]),
                Vec3::new(x[1], y, z[1]),
                Vec3::new(x[0], y, z[1]),
            );
        }

        /// A closed box.
        fn cuboid(&mut self, min: Vec3, max: Vec3) {
            let p = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
            let (a, b) = (min, max);
            self.quad(
                p(a.x, a.y, a.z),
                p(b.x, a.y, a.z),
                p(b.x, a.y, b.z),
                p(a.x, a.y, b.z),
            );
            self.quad(
                p(a.x, b.y, a.z),
                p(b.x, b.y, a.z),
                p(b.x, b.y, b.z),
                p(a.x, b.y, b.z),
            );
            self.quad(
                p(a.x, a.y, a.z),
                p(b.x, a.y, a.z),
                p(b.x, b.y, a.z),
                p(a.x, b.y, a.z),
            );
            self.quad(
                p(a.x, a.y, b.z),
                p(b.x, a.y, b.z),
                p(b.x, b.y, b.z),
                p(a.x, b.y, b.z),
            );
            self.quad(
                p(a.x, a.y, a.z),
                p(a.x, a.y, b.z),
                p(a.x, b.y, b.z),
                p(a.x, b.y, a.z),
            );
            self.quad(
                p(b.x, a.y, a.z),
                p(b.x, a.y, b.z),
                p(b.x, b.y, b.z),
                p(b.x, b.y, a.z),
            );
        }

        fn mesh(&self) -> ColliderMesh {
            ColliderMesh::from_triangles(self.vertices.clone(), self.triangles.clone()).unwrap()
        }
    }

    fn room(cols: u32, rows: u32) -> Soup {
        let mut soup = Soup::default();
        soup.floor([0.0, cols as f32], [0.0, rows as f32], 0.0);
        soup
    }

    fn grid_of(soup: &Soup) -> WorldGrid {
        world_grid(&soup.mesh(), &SplatTransform::IDENTITY, &unit(), None).unwrap()
    }

    #[test]
    fn a_bare_floor_is_all_open_and_its_bounds_are_the_room() {
        let grid = grid_of(&room(5, 4));
        assert_eq!((grid.cols(), grid.rows()), (5, 4));
        assert_eq!(
            grid.counts(),
            CellCounts {
                open: 20,
                blocked: 0,
                no_floor: 0
            }
        );
        assert_eq!(grid.origin(), [0.0, 0.0]);
        assert_eq!(
            grid.bounds(),
            WorldBounds {
                min: [0.0, 0.0],
                max: [5.0, 4.0]
            }
        );
        assert_eq!(grid.mesh_bounds(), [Vec3::ZERO, Vec3::new(5.0, 0.0, 4.0)]);
        assert_eq!(
            grid.largest_open_area(),
            Some(OpenArea {
                squares: 20,
                area_m2: 20.0,
                extent_squares: [5, 4]
            })
        );
        assert_eq!(grid.cell_centre(2, 3), Some([2.5, 3.5]));
        assert_eq!(grid.cell(5, 0), None);
        assert_eq!(grid.cell_centre(0, 4), None);
        assert_eq!(grid.cross_check(), None);
    }

    #[test]
    fn squares_follow_multiples_of_the_square_size_in_world_coordinates() {
        let mut soup = Soup::default();
        soup.floor([2.2, 5.3], [-1.4, 0.9], 0.0);
        let grid = grid_of(&soup);
        assert_eq!(grid.origin(), [2.0, -2.0]);
        assert_eq!((grid.cols(), grid.rows()), (4, 3));
        assert_eq!(grid.cell_centre(0, 0), Some([2.5, -1.5]));
    }

    #[test]
    fn furniture_in_the_hero_band_blocks_its_square_only() {
        let mut soup = room(5, 5);
        soup.cuboid(Vec3::new(2.3, 0.0, 1.3), Vec3::new(2.7, 1.0, 1.7));
        let grid = grid_of(&soup);
        assert_eq!(grid.cell(2, 1), Some(WorldCell::Blocked));
        assert_eq!(grid.counts().blocked, 1);
        assert_eq!(grid.counts().open, 24);
        // The 24 left over are still one connected area.
        assert_eq!(grid.largest_open_area().unwrap().squares, 24);
    }

    #[test]
    fn a_low_step_and_an_overhead_beam_do_not_block() {
        let mut soup = room(5, 5);
        // 15 cm high: a figure steps over it.
        soup.cuboid(Vec3::new(1.2, 0.0, 1.2), Vec3::new(1.8, 0.15, 1.8));
        // A beam from 2.0 to 2.2 m: overhead.
        soup.cuboid(Vec3::new(3.0, 2.0, 0.0), Vec3::new(3.3, 2.2, 5.0));
        let grid = grid_of(&soup);
        assert_eq!(grid.counts().blocked, 0, "{:?}", grid.counts());
        assert_eq!(grid.counts().open, 25);
    }

    #[test]
    fn a_platform_too_high_to_step_onto_blocks_and_a_pit_has_no_floor() {
        let mut soup = Soup::default();
        // A 4 x 3 room built from per-square floor patches so single squares can differ.
        for row in 0..3 {
            for col in 0..4 {
                let (x, z) = (col as f32, row as f32);
                let y = match (col, row) {
                    (1, 1) => 0.5,  // a platform half a metre up
                    (2, 1) => -0.6, // a pit
                    (3, 2) => 0.15, // slightly raised floor, inside the tolerance
                    _ => 0.0,
                };
                soup.floor([x, x + 1.0], [z, z + 1.0], y);
            }
        }
        let grid = grid_of(&soup);
        assert_eq!(grid.cell(1, 1), Some(WorldCell::Blocked));
        assert_eq!(grid.cell(2, 1), Some(WorldCell::NoFloor));
        assert_eq!(grid.cell(3, 2), Some(WorldCell::Open));
        assert_eq!(
            grid.counts(),
            CellCounts {
                open: 10,
                blocked: 1,
                no_floor: 1
            }
        );
    }

    #[test]
    fn a_hole_in_the_floor_is_no_floor_and_is_not_walkable() {
        let mut soup = Soup::default();
        soup.floor([0.0, 4.0], [0.0, 1.0], 0.0);
        soup.floor([0.0, 1.0], [1.0, 2.0], 0.0);
        soup.floor([2.0, 4.0], [1.0, 2.0], 0.0);
        soup.floor([0.0, 4.0], [2.0, 3.0], 0.0);
        let grid = grid_of(&soup);
        assert_eq!(grid.cell(1, 1), Some(WorldCell::NoFloor));
        assert_eq!(grid.counts().no_floor, 1);
        assert!(!grid.walkable_indices().contains(&5));
        // The ring around the hole is one connected area of 11 squares.
        assert_eq!(grid.largest_open_area().unwrap().squares, 11);
    }

    #[test]
    fn a_wall_splits_the_room_and_the_larger_side_is_the_largest_area() {
        let mut soup = room(5, 3);
        soup.cuboid(Vec3::new(2.3, 0.0, 0.0), Vec3::new(2.6, 1.0, 3.0));
        let grid = grid_of(&soup);
        assert_eq!(grid.counts().blocked, 3);
        assert_eq!(grid.counts().open, 12);
        // 2 x 3 on the left, 2 x 3 on the right: a tie keeps the first found (the left).
        assert_eq!(
            grid.largest_open_area(),
            Some(OpenArea {
                squares: 6,
                area_m2: 6.0,
                extent_squares: [2, 3]
            })
        );
        let mut wider = room(6, 3);
        wider.cuboid(Vec3::new(1.3, 0.0, 0.0), Vec3::new(1.6, 1.0, 3.0));
        let grid = grid_of(&wider);
        assert_eq!(grid.largest_open_area().unwrap().squares, 12);
        assert_eq!(grid.largest_open_area().unwrap().extent_squares, [4, 3]);
    }

    #[test]
    fn diagonal_neighbours_do_not_connect_open_areas() {
        let mut soup = room(2, 2);
        soup.cuboid(Vec3::new(0.3, 0.0, 1.3), Vec3::new(0.7, 1.0, 1.7));
        soup.cuboid(Vec3::new(1.3, 0.0, 0.3), Vec3::new(1.7, 1.0, 0.7));
        let grid = grid_of(&soup);
        // Open squares are (0,0) and (1,1), touching only at a corner.
        assert_eq!(grid.counts().open, 2);
        assert_eq!(grid.largest_open_area().unwrap().squares, 1);
    }

    #[test]
    fn a_grid_with_nothing_open_has_no_open_area() {
        let mut soup = room(1, 1);
        soup.cuboid(Vec3::new(0.2, 0.0, 0.2), Vec3::new(0.8, 1.0, 0.8));
        assert_eq!(grid_of(&soup).largest_open_area(), None);
    }

    #[test]
    fn the_square_size_is_configurable_and_five_feet_by_default() {
        assert_eq!(WorldGridSettings::default().cell_m, 1.524);
        let mut soup = Soup::default();
        soup.floor([0.0, 6.096], [0.0, 3.048], 0.0);
        let grid = world_grid(
            &soup.mesh(),
            &SplatTransform::IDENTITY,
            &WorldGridSettings::default(),
            None,
        )
        .unwrap();
        assert_eq!((grid.cols(), grid.rows()), (4, 2));
        assert_eq!(grid.counts().open, 8);
        let coarse = world_grid(
            &soup.mesh(),
            &SplatTransform::IDENTITY,
            &WorldGridSettings {
                cell_m: 3.048,
                ..WorldGridSettings::default()
            },
            None,
        )
        .unwrap();
        assert_eq!((coarse.cols(), coarse.rows()), (2, 1));
    }

    #[test]
    fn a_world_transform_is_applied_before_the_grid_is_read() {
        // A room in Marble's frame: y down, z forward, scaled by 2 and 1 m above the floor.
        // Raw floor at y = 0.5 is world y = 0; raw x 0..2, z -2..0 is world x 0..4, z 0..4.
        let mut raw = Soup::default();
        raw.floor([0.0, 2.0], [-2.0, 0.0], 0.5);
        // A box standing on that floor, 1 m high in the world: raw y 0.0..0.5.
        raw.cuboid(Vec3::new(0.5, 0.0, -1.2), Vec3::new(0.7, 0.5, -1.0));
        let transform = marble_world_transform(2.0, 1.0).unwrap();
        let grid = world_grid(&raw.mesh(), &transform, &unit(), None).unwrap();
        assert_eq!((grid.cols(), grid.rows()), (4, 4));
        // World x 1..1.4, z 2..2.4 is square (1, 2).
        assert_eq!(grid.cell(1, 2), Some(WorldCell::Blocked));
        assert_eq!(grid.counts().blocked, 1);
        assert_eq!(grid.counts().open, 15);
        // Without the transform the same mesh is upside down and nothing lines up.
        let untransformed =
            world_grid(&raw.mesh(), &SplatTransform::IDENTITY, &unit(), None).unwrap();
        assert_ne!(untransformed.counts(), grid.counts());
    }

    #[test]
    fn a_footprint_fixes_the_origin_and_leaves_out_the_mesh_beyond_it() {
        let mut soup = room(10, 4);
        // A box well outside the footprint, and one inside it.
        soup.cuboid(Vec3::new(7.3, 0.0, 1.3), Vec3::new(7.7, 1.0, 1.7));
        soup.cuboid(Vec3::new(2.8, 0.0, 1.3), Vec3::new(3.2, 1.0, 1.7));
        let settings = WorldGridSettings {
            footprint: Some(WorldBounds {
                min: [1.5, 0.5],
                max: [4.0, 3.0],
            }),
            ..unit()
        };
        let grid = world_grid(&soup.mesh(), &SplatTransform::IDENTITY, &settings, None).unwrap();
        assert_eq!(grid.origin(), [1.5, 0.5]);
        assert_eq!((grid.cols(), grid.rows()), (3, 3));
        // The near box, x 2.8 to 3.2 and z 1.3 to 1.7, straddles the squares (1, 0) and (1, 1)
        // of the shifted lattice and blocks both; the far box is outside and blocks nothing.
        assert_eq!(grid.cell(1, 0), Some(WorldCell::Blocked));
        assert_eq!(grid.cell(1, 1), Some(WorldCell::Blocked));
        assert_eq!(grid.counts().blocked, 2);
        assert_eq!(grid.counts().no_floor, 0);
        assert_eq!(grid.mesh_bounds()[1].x, 10.0);
    }

    #[test]
    fn the_same_inputs_give_the_same_grid() {
        let mut soup = room(7, 6);
        soup.cuboid(Vec3::new(3.1, 0.0, 2.2), Vec3::new(4.4, 1.2, 3.1));
        assert_eq!(grid_of(&soup), grid_of(&soup));
    }

    #[test]
    fn a_grid_read_from_a_glb_matches_one_read_from_the_triangles() {
        let mut soup = room(5, 4);
        soup.cuboid(Vec3::new(2.3, 0.0, 1.3), Vec3::new(2.7, 1.0, 1.7));
        let glb = test_glb::encode(&soup.vertices, &soup.triangles);
        let from_file = ColliderMesh::from_glb(&glb).unwrap();
        let grid = world_grid(&from_file, &SplatTransform::IDENTITY, &unit(), None).unwrap();
        assert_eq!(grid, grid_of(&soup));
    }

    #[test]
    fn open_squares_feed_the_battlefield_grid() {
        let mut soup = room(5, 4);
        soup.cuboid(Vec3::new(2.3, 0.0, 1.3), Vec3::new(2.7, 1.0, 1.7));
        let grid = grid_of(&soup);
        let battle = Grid::new(
            grid.origin(),
            grid.cell_m(),
            grid.cols(),
            grid.rows(),
            &grid.walkable_indices(),
        )
        .unwrap();
        assert!(battle.is_walkable(super::super::grid::Cell::new(0, 0)));
        assert!(!battle.is_walkable(super::super::grid::Cell::new(2, 1)));
    }

    /// Opaque splats of one size at the given positions, upright.
    fn splats_at(points: &[[f32; 3]], scale: [f32; 3]) -> Vec<RawSplat> {
        points
            .iter()
            .map(|p| RawSplat {
                position: Vec3::new(p[0], p[1], p[2]),
                scale: Vec3::new(scale[0], scale[1], scale[2]),
                rotation: Quat::IDENTITY,
                colour: [200, 180, 150, 255],
            })
            .collect()
    }

    fn ground_splats(size: u32) -> Vec<RawSplat> {
        let points: Vec<[f32; 3]> = (0..size * 5)
            .flat_map(|i| {
                (0..size * 5).map(move |j| [i as f32 * 0.2 + 0.1, 0.0, j as f32 * 0.2 + 0.1])
            })
            .collect();
        splats_at(&points, [0.2, 0.02, 0.2])
    }

    #[test]
    fn the_probe_can_block_a_square_the_mesh_left_open_and_never_unblocks_one() {
        // The mesh sees an empty 5 x 5 floor except square (0, 0), which has a box.
        let mut soup = room(5, 5);
        soup.cuboid(Vec3::new(0.3, 0.0, 0.3), Vec3::new(0.7, 1.0, 0.7));
        let mut splats = ground_splats(5);
        // An object the mesh missed, in the middle of square (3, 2), at hero height.
        splats.extend(splats_at(&[[3.5, 0.9, 2.5]], [0.15, 0.15, 0.15]));
        let grid = world_grid(
            &soup.mesh(),
            &SplatTransform::IDENTITY,
            &unit(),
            Some(&splats),
        )
        .unwrap();
        assert_eq!(grid.cell(3, 2), Some(WorldCell::Blocked));
        assert_eq!(grid.cell(0, 0), Some(WorldCell::Blocked));
        assert_eq!(
            grid.cross_check(),
            Some(CrossCheck {
                blocked_by_both: 0,
                blocked_by_probe_only: 1,
                blocked_by_mesh_only: 1
            })
        );
        // The ground splats alone block nothing.
        assert_eq!(grid.counts().blocked, 2);
        assert_eq!(grid.counts().open, 23);
    }

    #[test]
    fn a_square_both_sources_block_is_counted_as_agreement() {
        let mut soup = room(3, 3);
        soup.cuboid(Vec3::new(1.3, 0.0, 1.3), Vec3::new(1.7, 1.0, 1.7));
        let splats = splats_at(&[[1.3, 0.9, 1.3]], [0.1, 0.1, 0.1]);
        let grid = world_grid(
            &soup.mesh(),
            &SplatTransform::IDENTITY,
            &unit(),
            Some(&splats),
        )
        .unwrap();
        assert_eq!(
            grid.cross_check(),
            Some(CrossCheck {
                blocked_by_both: 1,
                blocked_by_probe_only: 0,
                blocked_by_mesh_only: 0
            })
        );
        assert_eq!(grid.counts().blocked, 1);
    }

    #[test]
    fn the_probe_uses_the_same_world_transform_as_the_mesh() {
        // Raw frame: 2 x scale, floor 1 m below the camera. A splat at raw (0.625, 0.0, -1.375)
        // is at world (1.25, 1.0, 2.75): square (1, 2) of a 4 x 4 room.
        let mut raw = Soup::default();
        raw.floor([0.0, 2.0], [-2.0, 0.0], 0.5);
        let transform = marble_world_transform(2.0, 1.0).unwrap();
        let splats = splats_at(&[[0.625, 0.0, -1.375]], [0.05, 0.05, 0.05]);
        let grid = world_grid(&raw.mesh(), &transform, &unit(), Some(&splats)).unwrap();
        assert_eq!(grid.cell(1, 2), Some(WorldCell::Blocked));
        assert_eq!(grid.counts().blocked, 1);
    }

    #[test]
    fn a_probe_volume_too_large_for_the_collider_is_a_typed_error() {
        let mut soup = Soup::default();
        soup.floor([0.0, 100.0], [0.0, 100.0], 0.0);
        let splats = splats_at(&[[50.0, 0.9, 50.0]], [0.1, 0.1, 0.1]);
        let result = world_grid(
            &soup.mesh(),
            &SplatTransform::IDENTITY,
            &WorldGridSettings::default(),
            Some(&splats),
        );
        assert_eq!(result, Err(WorldGridError::Probe(ColliderError::Bounds)));
        // The same room reads fine without the probe.
        assert!(
            world_grid(
                &soup.mesh(),
                &SplatTransform::IDENTITY,
                &WorldGridSettings::default(),
                None
            )
            .is_ok()
        );
    }

    #[test]
    fn bad_settings_a_bad_transform_and_oversize_worlds_are_refused() {
        let mesh = room(2, 2).mesh();
        let run = |settings: WorldGridSettings| {
            world_grid(&mesh, &SplatTransform::IDENTITY, &settings, None).unwrap_err()
        };
        let base = unit();
        for (settings, expected) in [
            (
                WorldGridSettings {
                    cell_m: 0.0,
                    ..base
                },
                WorldGridError::CellSize,
            ),
            (
                WorldGridSettings {
                    cell_m: f32::NAN,
                    ..base
                },
                WorldGridError::CellSize,
            ),
            (
                WorldGridSettings {
                    hero_band_m: [1.0, 0.5],
                    ..base
                },
                WorldGridError::HeroBand,
            ),
            (
                WorldGridSettings {
                    hero_band_m: [0.3, f32::INFINITY],
                    ..base
                },
                WorldGridError::HeroBand,
            ),
            (
                WorldGridSettings {
                    floor_tolerance_m: -0.1,
                    ..base
                },
                WorldGridError::FloorTolerance,
            ),
            (
                WorldGridSettings {
                    floor_normal_y_min: 0.0,
                    ..base
                },
                WorldGridError::FloorNormal,
            ),
            (
                WorldGridSettings {
                    floor_coverage: 1.5,
                    ..base
                },
                WorldGridError::FloorCoverage,
            ),
            (
                WorldGridSettings {
                    blocking_wall_share: 0.0,
                    ..base
                },
                WorldGridError::BlockingWallShare,
            ),
            (
                WorldGridSettings {
                    probe_voxel_m: 1.0,
                    ..base
                },
                WorldGridError::ProbeVoxel,
            ),
            (
                WorldGridSettings {
                    footprint: Some(WorldBounds {
                        min: [1.0, 0.0],
                        max: [1.0, 3.0],
                    }),
                    ..base
                },
                WorldGridError::Footprint,
            ),
            (
                WorldGridSettings {
                    footprint: Some(WorldBounds {
                        min: [f32::NAN, 0.0],
                        max: [1.0, 3.0],
                    }),
                    ..base
                },
                WorldGridError::Footprint,
            ),
        ] {
            assert_eq!(run(settings), expected);
        }
        let flat = SplatTransform {
            scale: 0.0,
            ..SplatTransform::IDENTITY
        };
        assert_eq!(
            world_grid(&mesh, &flat, &base, None),
            Err(WorldGridError::Transform)
        );
        let mut wide = Soup::default();
        wide.floor([0.0, 300.0], [0.0, 2.0], 0.0);
        assert_eq!(
            world_grid(&wide.mesh(), &SplatTransform::IDENTITY, &base, None),
            Err(WorldGridError::GridTooLarge)
        );
    }

    #[test]
    fn a_mesh_that_overflows_once_placed_is_refused() {
        let mesh = ColliderMesh::from_triangles(
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0e38, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
            vec![[0, 1, 2]],
        )
        .unwrap();
        let loud = SplatTransform {
            scale: 100.0,
            ..SplatTransform::IDENTITY
        };
        assert_eq!(
            world_grid(&mesh, &loud, &unit(), None),
            Err(WorldGridError::NonFiniteGeometry)
        );
    }
}
