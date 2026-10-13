//! Battlefield grid on the ground plane. Ported from the earlier build's `grid_math.go`:
//! cells are squares of `cell_m` metres whose corner `(0, 0)` sits at `origin` on the XZ plane;
//! columns grow along +X and rows along +Z. The grid is staging only: the rules engine has no
//! positions, so nothing here decides range, reach or movement outcomes.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use super::{
    layout::{Cover, Layout},
    math::Vec3,
    terrain::HeightField,
};

pub const MAX_GRID_SIDE: u32 = 64;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Cell {
    pub col: i32,
    pub row: i32,
}

impl Cell {
    pub const fn new(col: i32, row: i32) -> Self {
        Self { col, row }
    }

    /// Chebyshev distance: the number of eight-way steps between two cells.
    pub fn steps_to(self, other: Self) -> u32 {
        self.col
            .abs_diff(other.col)
            .max(self.row.abs_diff(other.row))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridError {
    EmptyGrid,
    SideTooLarge,
    CellSizeInvalid,
    OriginInvalid,
    WalkableIndexOutOfRange,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    origin: [f32; 2],
    cell_m: f32,
    cols: u32,
    rows: u32,
    walkable: Vec<bool>,
    /// Ground height under the grid, for scenes whose floor is not flat (a scanned place).
    terrain: Option<HeightField>,
    /// Height of the flat play plane (see [`Grid::with_terrain`]).
    play_plane_y: f32,
    /// Passable but not a place to stop (a low obstacle a figure can step over).
    no_stand: Vec<bool>,
    difficult: Vec<bool>,
    cover: Vec<Cover>,
    blocked_steps: BTreeSet<[Cell; 2]>,
    collision_ground: bool,
}

impl Grid {
    /// `walkable_indices` are `row * cols + col`, as in the earlier protocol.
    pub fn new(
        origin: [f32; 2],
        cell_m: f32,
        cols: u32,
        rows: u32,
        walkable_indices: &[u32],
    ) -> Result<Self, GridError> {
        if cols == 0 || rows == 0 {
            return Err(GridError::EmptyGrid);
        }
        if cols > MAX_GRID_SIDE || rows > MAX_GRID_SIDE {
            return Err(GridError::SideTooLarge);
        }
        if !cell_m.is_finite() || !(0.1..=20.0).contains(&cell_m) {
            return Err(GridError::CellSizeInvalid);
        }
        if !origin[0].is_finite() || !origin[1].is_finite() {
            return Err(GridError::OriginInvalid);
        }
        let mut walkable = vec![false; (cols * rows) as usize];
        for &index in walkable_indices {
            let slot = walkable
                .get_mut(index as usize)
                .ok_or(GridError::WalkableIndexOutOfRange)?;
            *slot = true;
        }
        let cells = walkable.len();
        Ok(Self {
            origin,
            cell_m,
            cols,
            rows,
            walkable,
            terrain: None,
            play_plane_y: 0.0,
            no_stand: vec![false; cells],
            difficult: vec![false; cells],
            cover: vec![Cover::None; cells],
            blocked_steps: BTreeSet::new(),
            collision_ground: false,
        })
    }

    /// A grid where every cell is walkable except the listed blocked ones.
    pub fn open_except(
        origin: [f32; 2],
        cell_m: f32,
        cols: u32,
        rows: u32,
        blocked: &[Cell],
    ) -> Result<Self, GridError> {
        let mut grid = Self::new(origin, cell_m, cols, rows, &[])?;
        grid.walkable.fill(true);
        for cell in blocked {
            if let Some(index) = grid.index(*cell) {
                grid.walkable[index] = false;
            }
        }
        Ok(grid)
    }

    /// The same grid on a scanned place. The grid, its lines and every figure share one flat play
    /// plane (a straight grid cannot follow a lumpy floor): its height is the median of the
    /// field's height under the walkable cell centres, which a few tall tufts or ditches do not
    /// move. [`Grid::plane_deviation_m`] says how far the field strays from it.
    pub fn with_terrain(mut self, terrain: HeightField) -> Self {
        let mut heights: Vec<f32> = self
            .cells_for_plane()
            .map(|cell| {
                let (x, z) = self.centre_xz(cell);
                terrain.height_at(x, z)
            })
            .collect();
        heights.sort_by(f32::total_cmp);
        self.play_plane_y = heights.get(heights.len() / 2).copied().unwrap_or(0.0);
        self.terrain = Some(terrain);
        self
    }

    /// The measured ground under the grid, when the scene has one.
    pub fn terrain(&self) -> Option<&HeightField> {
        self.terrain.as_ref()
    }

    /// Height of the flat play plane the grid and figures stand on (`0` unless the grid was fitted
    /// to a scan).
    pub fn play_plane_y(&self) -> f32 {
        self.play_plane_y
    }

    /// The largest distance, in metres, between the measured ground under a walkable cell centre
    /// and the play plane: how far a figure may float above or sink into the scan.
    pub fn plane_deviation_m(&self) -> f32 {
        let Some(terrain) = &self.terrain else {
            return 0.0;
        };
        self.cells_for_plane()
            .map(|cell| {
                let (x, z) = self.centre_xz(cell);
                (terrain.height_at(x, z) - self.play_plane_y).abs()
            })
            .fold(0.0, f32::max)
    }

    /// Ground height at a world point: the play plane everywhere (`0` on a flat grid).
    pub fn ground_height(&self, _x: f32, _z: f32) -> f32 {
        if self.collision_ground
            && let Some(terrain) = &self.terrain
        {
            return terrain.height_at(_x, _z);
        }
        self.play_plane_y
    }

    /// Scan-authored barriers and local ground heights; old scenes retain their play plane.
    pub fn with_collision(mut self, steps: &[[Cell; 2]]) -> Result<Self, GridError> {
        if steps.len() > (self.cols * self.rows * 4) as usize {
            return Err(GridError::WalkableIndexOutOfRange);
        }
        for &[from, to] in steps {
            if !self.in_bounds(from) || !self.in_bounds(to) || from.steps_to(to) != 1 {
                return Err(GridError::WalkableIndexOutOfRange);
            }
            self.blocked_steps.insert(ordered_step(from, to));
        }
        self.collision_ground = true;
        Ok(self)
    }

    pub fn step_clear(&self, from: Cell, to: Cell) -> bool {
        self.is_walkable(from)
            && self.is_walkable(to)
            && from.steps_to(to) == 1
            && !self.blocked_steps.contains(&ordered_step(from, to))
            && (from.col == to.col
                || from.row == to.row
                || (self.is_walkable(Cell::new(from.col, to.row))
                    && self.is_walkable(Cell::new(to.col, from.row))))
    }

    pub(crate) fn has_collision(&self) -> bool {
        self.collision_ground
    }

    /// A scan validates the capsule at cell centres and along the sweeps between neighbours
    /// with a clear step, and nowhere else. An offset a pose invents (a lunge, a recoil, a
    /// shake) is therefore kept only along such a sweep from a cell's centre, and no further
    /// than halfway; any other offset is dropped, so feet never move into a fence. The poses
    /// can still lean.
    pub(crate) fn constrain_figure(&self, point: Vec3, anchor: Vec3) -> Vec3 {
        if !self.collision_ground {
            return point;
        }
        let (x, z) = self
            .checked_offset(point, anchor)
            .map_or((anchor.x, anchor.z), |offset| {
                (anchor.x + offset[0], anchor.z + offset[1])
            });
        Vec3::new(x, self.ground_height(x, z), z)
    }

    /// The part of `point - anchor` that lies on a checked sweep; see [`Self::constrain_figure`].
    fn checked_offset(&self, point: Vec3, anchor: Vec3) -> Option<[f32; 2]> {
        let home = self.cell_at(anchor)?;
        let centre = self.cell_centre(home);
        let on_centre = (anchor.x - centre.x).abs().max((anchor.z - centre.z).abs()) < 1e-3;
        let offset = [point.x - anchor.x, point.z - anchor.z];
        let length = offset[0].hypot(offset[1]);
        if !on_centre || !length.is_finite() || length < 1e-4 {
            return None;
        }
        // The neighbour the offset points at, of the eight.
        let toward = Cell::new(
            home.col + (offset[0] / length).round() as i32,
            home.row + (offset[1] / length).round() as i32,
        );
        if toward == home || !self.step_clear(home, toward) {
            return None;
        }
        let neighbour = self.cell_centre(toward);
        let sweep = [neighbour.x - centre.x, neighbour.z - centre.z];
        let sweep_length = sweep[0].hypot(sweep[1]);
        let along = (offset[0] * sweep[0] + offset[1] * sweep[1]) / sweep_length;
        let kept = along.clamp(0.0, sweep_length * 0.5) / sweep_length;
        Some([sweep[0] * kept, sweep[1] * kept])
    }

    /// The walkable cells, or every cell when none is walkable.
    fn cells_for_plane(&self) -> impl Iterator<Item = Cell> + '_ {
        let any_walkable = self.walkable.iter().any(|walkable| *walkable);
        (0..self.rows as i32).flat_map(move |row| {
            (0..self.cols as i32)
                .map(move |col| Cell::new(col, row))
                .filter(move |cell| !any_walkable || self.is_walkable(*cell))
        })
    }

    fn centre_xz(&self, cell: Cell) -> (f32, f32) {
        (
            self.origin[0] + (cell.col as f32 + 0.5) * self.cell_m,
            self.origin[1] + (cell.row as f32 + 0.5) * self.cell_m,
        )
    }

    /// This grid with a layout's obstacles applied: blocking obstacles remove cells from
    /// movement, every obstacle gives its cover to the cell it stands on, and difficult ground is
    /// marked. A low non-blocking obstacle can be crossed but not stopped on.
    pub fn with_layout(&self, layout: &Layout) -> Self {
        let mut grid = self.clone();
        for obstacle in &layout.obstacles {
            for cell in &obstacle.cells {
                let Some(index) = grid.index(*cell) else {
                    continue;
                };
                if obstacle.blocks_movement {
                    grid.walkable[index] = false;
                } else {
                    grid.no_stand[index] = true;
                }
                grid.cover[index] = obstacle.cover;
            }
        }
        for cell in &layout.difficult {
            if let Some(index) = grid.index(*cell) {
                grid.difficult[index] = true;
            }
        }
        grid
    }

    /// Whether a figure may end a move here.
    pub fn is_standable(&self, cell: Cell) -> bool {
        self.index(cell)
            .is_some_and(|index| self.walkable[index] && !self.no_stand[index])
    }

    pub fn is_difficult(&self, cell: Cell) -> bool {
        self.index(cell).is_some_and(|index| self.difficult[index])
    }

    pub fn cover_at(&self, cell: Cell) -> Cover {
        self.index(cell)
            .map_or(Cover::None, |index| self.cover[index])
    }

    /// The best cover next to `cell` (eight-way) and the neighbouring cell that gives it; ties
    /// break toward the lower row then column so the answer is stable.
    pub fn cover_beside(&self, cell: Cell) -> Option<(Cell, Cover)> {
        NEIGHBOURS
            .iter()
            .map(|(delta_col, delta_row)| Cell::new(cell.col + delta_col, cell.row + delta_row))
            .filter(|neighbour| self.cover_at(*neighbour) != Cover::None)
            .map(|neighbour| (self.cover_at(neighbour), neighbour))
            .max_by_key(|(cover, neighbour)| {
                // Strongest cover first, then a straight-on neighbour over a diagonal one.
                let diagonal = neighbour.col != cell.col && neighbour.row != cell.row;
                (
                    *cover,
                    Reverse(diagonal),
                    Reverse(neighbour.row),
                    Reverse(neighbour.col),
                )
            })
            .map(|(cover, neighbour)| (neighbour, cover))
    }

    /// Every cell reachable from `from` over walkable cells by the same moves as [`Grid::path`].
    pub fn reachable_from(&self, from: Cell) -> BTreeSet<Cell> {
        let mut seen = BTreeSet::new();
        if !self.is_walkable(from) {
            return seen;
        }
        let mut stack = vec![from];
        seen.insert(from);
        while let Some(current) = stack.pop() {
            for next in self.steps_from(current) {
                if seen.insert(next) {
                    stack.push(next);
                }
            }
        }
        seen
    }

    /// The walkable cells one move from `current`; a diagonal may not cut a blocked corner.
    fn steps_from(&self, current: Cell) -> impl Iterator<Item = Cell> + '_ {
        NEIGHBOURS.iter().filter_map(move |(delta_col, delta_row)| {
            let next = Cell::new(current.col + delta_col, current.row + delta_row);
            if !self.step_clear(current, next) {
                return None;
            }
            let diagonal = *delta_col != 0 && *delta_row != 0;
            let corners_open = self.is_walkable(Cell::new(current.col + delta_col, current.row))
                && self.is_walkable(Cell::new(current.col, current.row + delta_row));
            (!diagonal || corners_open).then_some(next)
        })
    }

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

    fn index(&self, cell: Cell) -> Option<usize> {
        self.in_bounds(cell)
            .then(|| cell.row as usize * self.cols as usize + cell.col as usize)
    }

    pub fn in_bounds(&self, cell: Cell) -> bool {
        cell.col >= 0
            && cell.row >= 0
            && (cell.col as u32) < self.cols
            && (cell.row as u32) < self.rows
    }

    pub fn is_walkable(&self, cell: Cell) -> bool {
        self.index(cell).is_some_and(|index| self.walkable[index])
    }

    pub fn walkable_cells(&self) -> impl Iterator<Item = Cell> + '_ {
        (0..self.rows as i32).flat_map(move |row| {
            (0..self.cols as i32)
                .map(move |col| Cell::new(col, row))
                .filter(|cell| self.is_walkable(*cell))
        })
    }

    /// The centre of the cell on the ground (`y` = 0 unless the grid has terrain).
    pub fn cell_centre(&self, cell: Cell) -> Vec3 {
        let x = self.origin[0] + (cell.col as f32 + 0.5) * self.cell_m;
        let z = self.origin[1] + (cell.row as f32 + 0.5) * self.cell_m;
        Vec3::new(x, self.ground_height(x, z), z)
    }

    /// The cell under a ground-plane point, or `None` outside the grid.
    pub fn cell_at(&self, point: Vec3) -> Option<Cell> {
        let col = ((point.x - self.origin[0]) / self.cell_m).floor();
        let row = ((point.z - self.origin[1]) / self.cell_m).floor();
        if !col.is_finite() || !row.is_finite() {
            return None;
        }
        let cell = Cell::new(col as i32, row as i32);
        self.in_bounds(cell).then_some(cell)
    }

    /// The middle of the whole grid on the ground.
    pub fn centre(&self) -> Vec3 {
        let x = self.origin[0] + self.cols as f32 * self.cell_m * 0.5;
        let z = self.origin[1] + self.rows as f32 * self.cell_m * 0.5;
        Vec3::new(x, self.ground_height(x, z), z)
    }

    /// Shortest eight-way path between two walkable cells, both ends included, found with A*
    /// (straight moves cost 10, diagonal 14, so routes look natural rather than staircase-like).
    /// Diagonal steps may not cut a blocked corner. Ties resolve by cell order, so the same grid
    /// always gives the same route.
    pub fn path(&self, from: Cell, to: Cell) -> Option<Vec<Cell>> {
        if !self.is_walkable(from) || !self.is_walkable(to) {
            return None;
        }
        let heuristic = |cell: Cell| {
            let (dc, dr) = (cell.col.abs_diff(to.col), cell.row.abs_diff(to.row));
            10 * dc.max(dr) + 4 * dc.min(dr)
        };
        let mut open = BinaryHeap::from([Reverse((heuristic(from), 0u32, from))]);
        let mut cost = BTreeMap::from([(from, 0u32)]);
        let mut parents = BTreeMap::from([(from, from)]);
        while let Some(Reverse((_, spent, current))) = open.pop() {
            if current == to {
                return Some(unwind(&parents, from, to));
            }
            if cost.get(&current).is_some_and(|best| *best < spent) {
                continue;
            }
            for next in self.steps_from(current) {
                let diagonal = next.col != current.col && next.row != current.row;
                let total = spent + if diagonal { 14 } else { 10 };
                if cost.get(&next).is_none_or(|best| total < *best) {
                    cost.insert(next, total);
                    parents.insert(next, current);
                    open.push(Reverse((total + heuristic(next), total, next)));
                }
            }
        }
        None
    }

    /// The walkable cell next to `target` that is not in `occupied` and is the closest to
    /// `from` by path length; ties break toward the lower row then column so it is stable.
    pub fn free_cell_beside(&self, target: Cell, from: Cell, occupied: &[Cell]) -> Option<Cell> {
        NEIGHBOURS
            .iter()
            .map(|(delta_col, delta_row)| Cell::new(target.col + delta_col, target.row + delta_row))
            .filter(|cell| self.is_standable(*cell) && !occupied.contains(cell))
            .filter_map(|cell| {
                let length = if cell == from {
                    0
                } else {
                    self.path(from, cell)?.len()
                };
                Some((length, cell.row, cell.col, cell))
            })
            .min()
            .map(|(_, _, _, cell)| cell)
    }
}

fn ordered_step(from: Cell, to: Cell) -> [Cell; 2] {
    if from < to { [from, to] } else { [to, from] }
}

const NEIGHBOURS: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

fn unwind(parents: &BTreeMap<Cell, Cell>, from: Cell, to: Cell) -> Vec<Cell> {
    let mut path = vec![to];
    let mut current = to;
    while current != from {
        let Some(parent) = parents.get(&current) else {
            break;
        };
        current = *parent;
        path.push(current);
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        Grid::open_except([-3.0, -2.0], 1.5, 4, 3, &[Cell::new(1, 1)]).unwrap()
    }

    /// A 12 by 10 field one metre apart over the test grid: flat at 0.4 m with one 1.2 m tuft.
    fn field(tuft_m: f32) -> HeightField {
        let mut heights = vec![0.4; 12 * 10];
        heights[5 * 12 + 5] = tuft_m;
        HeightField::new([-6.0, -5.0], 1.0, 12, 10, heights).unwrap()
    }

    #[test]
    fn terrain_gives_one_flat_play_plane_at_the_median_ground_height() {
        let grid = grid().with_terrain(field(1.2));
        assert!(
            (grid.play_plane_y() - 0.4).abs() < 1e-6,
            "tufts do not move the plane"
        );
        for cell in grid.walkable_cells() {
            assert_eq!(grid.cell_centre(cell).y, grid.play_plane_y());
        }
        assert_eq!(grid.ground_height(100.0, -100.0), grid.play_plane_y());
        assert_eq!(grid.centre().y, grid.play_plane_y());
        assert!(grid.plane_deviation_m() <= 0.8 + 1e-5);
    }

    #[test]
    fn a_grid_without_terrain_is_on_the_zero_plane_with_no_deviation() {
        let grid = grid();
        assert_eq!(grid.play_plane_y(), 0.0);
        assert_eq!(grid.plane_deviation_m(), 0.0);
        assert!(grid.terrain().is_none());
    }

    #[test]
    fn cell_centres_and_world_lookup_round_trip() {
        let grid = grid();
        for cell in grid.walkable_cells() {
            assert_eq!(grid.cell_at(grid.cell_centre(cell)), Some(cell));
        }
        let centre = grid.cell_centre(Cell::new(0, 0));
        assert_eq!((centre.x, centre.z), (-3.0 + 0.75, -2.0 + 0.75));
        assert_eq!(grid.cell_at(Vec3::new(100.0, 0.0, 0.0)), None);
    }

    #[test]
    fn blocked_and_out_of_range_cells_are_not_walkable() {
        let grid = grid();
        assert!(!grid.is_walkable(Cell::new(1, 1)));
        assert!(!grid.is_walkable(Cell::new(-1, 0)));
        assert!(!grid.is_walkable(Cell::new(4, 0)));
        assert!(grid.is_walkable(Cell::new(3, 2)));
        assert_eq!(grid.walkable_cells().count(), 11);
    }

    #[test]
    fn walkable_indices_use_row_major_order() {
        let grid = Grid::new([0.0, 0.0], 1.0, 3, 2, &[0, 5]).unwrap();
        assert!(grid.is_walkable(Cell::new(0, 0)));
        assert!(grid.is_walkable(Cell::new(2, 1)));
        assert_eq!(grid.walkable_cells().count(), 2);
        assert_eq!(
            Grid::new([0.0, 0.0], 1.0, 3, 2, &[6]),
            Err(GridError::WalkableIndexOutOfRange)
        );
    }

    #[test]
    fn invalid_grids_are_rejected_with_a_typed_error() {
        assert_eq!(
            Grid::new([0.0; 2], 1.0, 0, 3, &[]),
            Err(GridError::EmptyGrid)
        );
        assert_eq!(
            Grid::new([0.0; 2], 1.0, 65, 3, &[]),
            Err(GridError::SideTooLarge)
        );
        assert_eq!(
            Grid::new([0.0; 2], 0.0, 3, 3, &[]),
            Err(GridError::CellSizeInvalid)
        );
        assert_eq!(
            Grid::new([f32::NAN, 0.0], 1.0, 3, 3, &[]),
            Err(GridError::OriginInvalid)
        );
    }

    #[test]
    fn path_goes_around_a_blocked_cell_without_cutting_corners() {
        let grid = Grid::open_except([0.0, 0.0], 1.0, 3, 3, &[Cell::new(1, 1)]).unwrap();
        let path = grid.path(Cell::new(0, 0), Cell::new(2, 2)).unwrap();
        assert_eq!(path.first(), Some(&Cell::new(0, 0)));
        assert_eq!(path.last(), Some(&Cell::new(2, 2)));
        assert_eq!(path.len(), 5);
        assert!(!path.contains(&Cell::new(1, 1)));
        for pair in path.windows(2) {
            assert_eq!(pair[0].steps_to(pair[1]), 1);
        }
    }

    #[test]
    fn path_is_absent_when_an_end_is_blocked_or_unreachable() {
        let grid = Grid::open_except([0.0, 0.0], 1.0, 3, 1, &[Cell::new(1, 0)]).unwrap();
        assert!(grid.path(Cell::new(0, 0), Cell::new(2, 0)).is_none());
        assert!(grid.path(Cell::new(0, 0), Cell::new(1, 0)).is_none());
    }

    #[test]
    fn free_cell_beside_skips_occupied_cells_and_is_deterministic() {
        let grid = Grid::open_except([0.0, 0.0], 1.0, 5, 5, &[]).unwrap();
        let target = Cell::new(2, 1);
        let from = Cell::new(2, 4);
        let first = grid.free_cell_beside(target, from, &[target]).unwrap();
        assert_eq!(first.steps_to(target), 1);
        let second = grid
            .free_cell_beside(target, from, &[target, first])
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(second.steps_to(target), 1);
        assert_eq!(grid.free_cell_beside(target, from, &[target]), Some(first));
    }
}

#[cfg(test)]
mod scan_collision_tests {
    use super::*;

    #[test]
    fn scan_steps_detour_in_both_directions_and_figures_follow_local_ground() {
        let terrain = HeightField::new(
            [0.0, 0.0],
            1.0,
            4,
            3,
            vec![0.0, 0.2, 0.4, 0.6, 0.0, 0.2, 0.4, 0.6, 0.0, 0.2, 0.4, 0.6],
        )
        .unwrap();
        let a = Cell::new(1, 0);
        let b = Cell::new(2, 0);
        let grid = Grid::open_except([0.0, 0.0], 1.0, 3, 2, &[])
            .unwrap()
            .with_terrain(terrain)
            .with_collision(&[[a, b]])
            .unwrap();
        assert!(!grid.step_clear(a, b));
        assert!(!grid.step_clear(b, a));
        assert!(grid.path(a, b).unwrap().len() > 2);
        assert!((grid.cell_centre(Cell::new(0, 0)).y - 0.1).abs() < 1e-5);
        let home = grid.cell_centre(a);
        let invented = Vec3::new(2.9, 20.0, -3.0);
        assert_eq!(grid.constrain_figure(invented, home), home);
        // Toward the barrier: dropped, however small. Away from it, along a clear step: kept on
        // the line between the two centres, on the local ground, and never past halfway.
        let toward_fence = Vec3::new(home.x + 0.2, home.y, home.z);
        assert_eq!(grid.constrain_figure(toward_fence, home), home);
        let lunge = grid.constrain_figure(Vec3::new(home.x - 0.3, 9.0, home.z + 0.05), home);
        assert!((lunge.x - (home.x - 0.3)).abs() < 1e-5 && lunge.z == home.z);
        assert_eq!(lunge.y, grid.ground_height(lunge.x, lunge.z));
        let far = grid.constrain_figure(Vec3::new(home.x - 5.0, 0.0, home.z), home);
        assert!((far.x - (home.x - 0.5)).abs() < 1e-5);
        // Off a centre (mid-walk) nothing is added: the walk itself is the checked line.
        let walking = Vec3::new(home.x - 0.25, home.y, home.z);
        let shown = grid.constrain_figure(Vec3::new(walking.x, 0.0, walking.z + 0.2), walking);
        assert_eq!((shown.x, shown.z), (walking.x, walking.z));
    }
}
