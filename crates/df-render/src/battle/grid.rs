//! Battlefield grid on the ground plane. Ported from the earlier build's `grid_math.go`:
//! cells are squares of `cell_m` metres whose corner `(0, 0)` sits at `origin` on the XZ plane;
//! columns grow along +X and rows along +Z. The grid is staging only: the rules engine has no
//! positions, so nothing here decides range, reach or movement outcomes.

use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::cmp::Reverse;

use super::{layout::{Cover, Layout}, math::Vec3};

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
    /// Passable but not a place to stop (a low obstacle a figure can step over).
    no_stand: Vec<bool>,
    difficult: Vec<bool>,
    cover: Vec<Cover>,
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
            no_stand: vec![false; cells],
            difficult: vec![false; cells],
            cover: vec![Cover::None; cells],
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
        self.index(cell).map_or(Cover::None, |index| self.cover[index])
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
                (*cover, Reverse(diagonal), Reverse(neighbour.row), Reverse(neighbour.col))
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
            if !self.is_walkable(next) {
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

    /// The centre of the cell on the ground plane (`y` = 0).
    pub fn cell_centre(&self, cell: Cell) -> Vec3 {
        Vec3::new(
            self.origin[0] + (cell.col as f32 + 0.5) * self.cell_m,
            0.0,
            self.origin[1] + (cell.row as f32 + 0.5) * self.cell_m,
        )
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

    /// The middle of the whole grid on the ground plane.
    pub fn centre(&self) -> Vec3 {
        Vec3::new(
            self.origin[0] + self.cols as f32 * self.cell_m * 0.5,
            0.0,
            self.origin[1] + self.rows as f32 * self.cell_m * 0.5,
        )
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
