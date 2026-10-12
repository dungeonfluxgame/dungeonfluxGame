//! Derived staging formation. The rules engine has no positions, movement or range today (one
//! melee enemy, everyone in reach), so a client invents a stable layout purely to give the
//! picture somewhere to stand: heroes by seat on the near side (high rows, toward the camera),
//! enemies opposite on the far side, all on walkable cells. It is deterministic for a given grid
//! and head count and is never read back as game state. Server-supplied cells replace it by
//! building [`Token`](super::Token)s with those cells directly; the renderer does not change.

use super::grid::{Cell, Grid};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Formation {
    pub heroes: Vec<Cell>,
    pub enemies: Vec<Cell>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormationError {
    NoRoom,
}

/// Lays out `heroes` and `enemies` seats. Seat order is left to right as seen from the camera.
pub fn derive(grid: &Grid, heroes: usize, enemies: usize) -> Result<Formation, FormationError> {
    let rows = grid.rows() as i32;
    let near_row = (rows - 1 - rows / 5).max(0);
    let far_row = (rows / 5).min(rows - 1);
    let mut taken = Vec::new();
    let hero_cells = seat_row(grid, heroes, near_row, 2, &mut taken)?;
    let enemy_cells = seat_row(grid, enemies, far_row, 3, &mut taken)?;
    Ok(Formation {
        heroes: hero_cells,
        enemies: enemy_cells,
    })
}

/// Spreads `count` seats across a row, `gap` columns apart and centred on the grid, taking the
/// nearest free walkable cell to each wanted position.
fn seat_row(
    grid: &Grid,
    count: usize,
    row: i32,
    gap: i32,
    taken: &mut Vec<Cell>,
) -> Result<Vec<Cell>, FormationError> {
    let centre = (grid.cols() as i32 - 1) as f32 * 0.5;
    let mut seats = Vec::with_capacity(count);
    for index in 0..count {
        let offset = (index as f32 - (count as f32 - 1.0) * 0.5) * gap as f32;
        let wanted = (centre + offset, row as f32);
        let cell = grid
            .walkable_cells()
            .filter(|cell| !taken.contains(cell))
            .min_by(|a, b| {
                let distance = |cell: &Cell| {
                    let dc = cell.col as f32 - wanted.0;
                    let dr = cell.row as f32 - wanted.1;
                    // Rows matter more than columns so a seat stays on its side of the field.
                    dc * dc + dr * dr * 4.0
                };
                distance(a)
                    .total_cmp(&distance(b))
                    .then(a.row.cmp(&b.row))
                    .then(a.col.cmp(&b.col))
            })
            .ok_or(FormationError::NoRoom)?;
        taken.push(cell);
        seats.push(cell);
    }
    Ok(seats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        Grid::open_except([-6.0, -4.5], 1.5, 8, 6, &[Cell::new(3, 3), Cell::new(4, 2)]).unwrap()
    }

    #[test]
    fn heroes_stand_nearer_the_camera_than_the_enemy_on_walkable_distinct_cells() {
        let grid = grid();
        let formation = derive(&grid, 2, 1).unwrap();
        assert_eq!(formation.heroes.len(), 2);
        assert_eq!(formation.enemies.len(), 1);
        let hero_row = formation.heroes.iter().map(|cell| cell.row).min().unwrap();
        let enemy_row = formation.enemies.iter().map(|cell| cell.row).max().unwrap();
        assert!(hero_row > enemy_row);
        let all: Vec<Cell> = formation
            .heroes
            .iter()
            .chain(&formation.enemies)
            .copied()
            .collect();
        for (index, cell) in all.iter().enumerate() {
            assert!(grid.is_walkable(*cell));
            assert!(!all[..index].contains(cell));
        }
    }

    #[test]
    fn seats_run_left_to_right_by_seat_order() {
        let formation = derive(&grid(), 2, 1).unwrap();
        assert!(formation.heroes[0].col < formation.heroes[1].col);
    }

    #[test]
    fn the_formation_is_stable_for_the_same_input() {
        assert_eq!(derive(&grid(), 2, 1), derive(&grid(), 2, 1));
    }

    #[test]
    fn a_blocked_seat_moves_to_the_nearest_walkable_cell() {
        let blocked_everywhere_near =
            Grid::open_except([0.0, 0.0], 1.5, 5, 5, &[Cell::new(2, 3), Cell::new(1, 3)]).unwrap();
        let formation = derive(&blocked_everywhere_near, 1, 1).unwrap();
        assert!(blocked_everywhere_near.is_walkable(formation.heroes[0]));
    }

    #[test]
    fn too_many_combatants_for_the_grid_is_an_error() {
        let tiny = Grid::open_except([0.0, 0.0], 1.5, 2, 1, &[]).unwrap();
        assert_eq!(derive(&tiny, 2, 1), Err(FormationError::NoRoom));
    }
}
