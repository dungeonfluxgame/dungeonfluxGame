//! Battlefield layouts: obstacles, difficult ground and spawn cells on a scene's grid, authored
//! as data in `content/battle-scenes/<location-id>.json` (schema v2, `"layouts"`). The same files
//! are meant to be read by the tactical rules module, so the shapes here are the contract: a
//! layout is chosen per fight, heroes and enemies start on its spawn cells, blocking obstacles
//! remove cells from movement and give cover.
//!
//! The renderer only stages. Whatever the server decides about a fight (the layout, each
//! combatant's cell, a movement path) replaces what is derived here without any renderer change.

use std::collections::BTreeSet;

use super::grid::{Cell, Grid};

/// Cover an obstacle gives to a combatant standing next to it. Ordered weakest to strongest.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Cover {
    None,
    Half,
    ThreeQuarters,
    Total,
}

impl Cover {
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "none" => Self::None,
            "half" => Self::Half,
            "three-quarters" => Self::ThreeQuarters,
            "total" => Self::Total,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Obstacle {
    pub id: String,
    pub kind: String,
    pub cells: Vec<Cell>,
    pub blocks_movement: bool,
    pub cover: Cover,
    /// Prop art stem under `assets/ui/generated/`.
    pub sprite: String,
    pub height_m: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub id: String,
    pub label: String,
    pub obstacles: Vec<Obstacle>,
    pub difficult: Vec<Cell>,
    pub heroes: Vec<Cell>,
    pub enemies: Vec<Cell>,
}

pub const MIN_SPAWNS: usize = 4;
pub const MAX_OBSTACLE_HEIGHT_M: f32 = 4.0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutError {
    Identifier(String),
    DuplicateLayout(String),
    DuplicateObstacle(String),
    ObstacleHeight(String),
    OutsideGrid(String, Cell),
    ObstacleOverlap(String, Cell),
    EmptyObstacle(String),
    TooFewSpawns(String),
    DuplicateSpawn(String, Cell),
    SpawnBlocked(String, Cell),
    DifficultBlocked(String, Cell),
    Unreachable(String, Cell, Cell),
}

impl Layout {
    /// Checks the layout against the scene's base grid: every cell inside the grid, obstacles
    /// disjoint, spawns on free walkable cells, difficult ground that can be entered, and every
    /// hero spawn able to reach every enemy spawn once the blocking obstacles are placed.
    pub fn validate(&self, base: &Grid) -> Result<(), LayoutError> {
        let id = &self.id;
        let inside = |cell: &Cell| {
            base.in_bounds(*cell)
                .then_some(())
                .ok_or_else(|| LayoutError::OutsideGrid(id.clone(), *cell))
        };
        let mut obstacle_ids = BTreeSet::new();
        let mut occupied = BTreeSet::new();
        let mut blocking = BTreeSet::new();
        for obstacle in &self.obstacles {
            if !obstacle_ids.insert(obstacle.id.as_str()) {
                return Err(LayoutError::DuplicateObstacle(obstacle.id.clone()));
            }
            if obstacle.cells.is_empty() {
                return Err(LayoutError::EmptyObstacle(obstacle.id.clone()));
            }
            if !(0.1..=MAX_OBSTACLE_HEIGHT_M).contains(&obstacle.height_m) {
                return Err(LayoutError::ObstacleHeight(obstacle.id.clone()));
            }
            for cell in &obstacle.cells {
                inside(cell)?;
                if !occupied.insert(*cell) {
                    return Err(LayoutError::ObstacleOverlap(obstacle.id.clone(), *cell));
                }
                if obstacle.blocks_movement {
                    blocking.insert(*cell);
                }
            }
        }
        for cell in &self.difficult {
            inside(cell)?;
            if blocking.contains(cell) || !base.is_walkable(*cell) {
                return Err(LayoutError::DifficultBlocked(id.clone(), *cell));
            }
        }
        for spawns in [&self.heroes, &self.enemies] {
            if spawns.len() < MIN_SPAWNS {
                return Err(LayoutError::TooFewSpawns(id.clone()));
            }
        }
        let mut taken = BTreeSet::new();
        for cell in self.heroes.iter().chain(&self.enemies) {
            inside(cell)?;
            if !taken.insert(*cell) {
                return Err(LayoutError::DuplicateSpawn(id.clone(), *cell));
            }
            if occupied.contains(cell) || !base.is_walkable(*cell) {
                return Err(LayoutError::SpawnBlocked(id.clone(), *cell));
            }
        }
        let grid = base.with_layout(self);
        for hero in &self.heroes {
            let reachable = grid.reachable_from(*hero);
            if let Some(enemy) = self.enemies.iter().find(|enemy| !reachable.contains(enemy)) {
                return Err(LayoutError::Unreachable(id.clone(), *hero, *enemy));
            }
        }
        Ok(())
    }
}

/// FNV-1a over the key: a stable hash that does not depend on the platform or the process.
fn hash(key: &str) -> u64 {
    key.bytes().fold(0xcbf2_9ce4_8422_2325, |state, byte| {
        (state ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Picks the layout for a fight from data the view already carries (the scene and participant
/// identifiers, never the round), so a reload and the preview show the same layout for the same
/// fight. This is the one place the server's own choice replaces.
pub fn choose<'a>(layouts: &'a [Layout], key: &str) -> Option<&'a Layout> {
    if layouts.is_empty() {
        return None;
    }
    layouts.get(hash(key) as usize % layouts.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Grid {
        Grid::open_except([0.0, 0.0], 1.0, 8, 6, &[Cell::new(0, 0)]).unwrap()
    }

    fn crate_at(id: &str, cells: &[(i32, i32)]) -> Obstacle {
        Obstacle {
            id: id.to_owned(),
            kind: "crate".to_owned(),
            cells: cells.iter().map(|(c, r)| Cell::new(*c, *r)).collect(),
            blocks_movement: true,
            cover: Cover::Half,
            sprite: "prop-crates".to_owned(),
            height_m: 1.1,
        }
    }

    fn layout() -> Layout {
        Layout {
            id: "test".to_owned(),
            label: "Test".to_owned(),
            obstacles: vec![crate_at("a", &[(3, 2)]), crate_at("b", &[(4, 3), (4, 2)])],
            difficult: vec![Cell::new(2, 2)],
            heroes: (1..5).map(|c| Cell::new(c, 5)).collect(),
            enemies: (1..5).map(|c| Cell::new(c, 0)).collect(),
        }
    }

    #[test]
    fn a_well_formed_layout_validates() {
        assert_eq!(layout().validate(&base()), Ok(()));
    }

    #[test]
    fn cells_outside_the_grid_are_rejected() {
        let mut bad = layout();
        bad.obstacles[0].cells = vec![Cell::new(8, 2)];
        assert_eq!(
            bad.validate(&base()),
            Err(LayoutError::OutsideGrid("test".into(), Cell::new(8, 2)))
        );
        let mut bad = layout();
        bad.heroes[0] = Cell::new(-1, 5);
        assert!(matches!(
            bad.validate(&base()),
            Err(LayoutError::OutsideGrid(..))
        ));
    }

    #[test]
    fn a_spawn_on_an_obstacle_or_a_hole_is_rejected() {
        let mut bad = layout();
        bad.enemies[0] = Cell::new(3, 2);
        assert!(matches!(
            bad.validate(&base()),
            Err(LayoutError::SpawnBlocked(..))
        ));
        let mut bad = layout();
        bad.heroes[0] = Cell::new(0, 0);
        assert!(matches!(
            bad.validate(&base()),
            Err(LayoutError::SpawnBlocked(..))
        ));
    }

    #[test]
    fn duplicate_ids_overlaps_and_short_spawn_lists_are_rejected() {
        let mut bad = layout();
        bad.obstacles[1].id = "a".into();
        assert!(matches!(
            bad.validate(&base()),
            Err(LayoutError::DuplicateObstacle(_))
        ));
        let mut bad = layout();
        bad.obstacles[1].cells.push(Cell::new(3, 2));
        assert!(matches!(
            bad.validate(&base()),
            Err(LayoutError::ObstacleOverlap(..))
        ));
        let mut bad = layout();
        bad.heroes.pop();
        assert!(matches!(
            bad.validate(&base()),
            Err(LayoutError::TooFewSpawns(_))
        ));
        let mut bad = layout();
        bad.enemies[1] = bad.enemies[0];
        assert!(matches!(
            bad.validate(&base()),
            Err(LayoutError::DuplicateSpawn(..))
        ));
    }

    #[test]
    fn a_wall_that_cuts_the_field_makes_the_spawns_unreachable() {
        let mut bad = layout();
        bad.obstacles = vec![crate_at(
            "wall",
            &(0..8).map(|c| (c, 3)).collect::<Vec<_>>(),
        )];
        bad.difficult.clear();
        assert!(matches!(
            bad.validate(&base()),
            Err(LayoutError::Unreachable(..))
        ));
        // A one-cell gap restores the route.
        let mut gap = bad.clone();
        gap.obstacles[0].cells.retain(|cell| cell.col != 5);
        assert_eq!(gap.validate(&base()), Ok(()));
    }

    #[test]
    fn two_diagonal_obstacles_do_not_form_a_passable_gap() {
        // The rules never let a move cut a blocked corner, so reachability must not either.
        let grid =
            Grid::open_except([0.0, 0.0], 1.0, 2, 2, &[Cell::new(1, 0), Cell::new(0, 1)]).unwrap();
        let reachable = grid.reachable_from(Cell::new(0, 0));
        assert!(!reachable.contains(&Cell::new(1, 1)));
        assert_eq!(reachable.len(), 1);
    }

    #[test]
    fn difficult_ground_cannot_sit_under_a_blocking_obstacle() {
        let mut bad = layout();
        bad.difficult.push(Cell::new(3, 2));
        assert!(matches!(
            bad.validate(&base()),
            Err(LayoutError::DifficultBlocked(..))
        ));
    }

    #[test]
    fn obstacle_height_must_be_sensible() {
        let mut bad = layout();
        bad.obstacles[0].height_m = 9.0;
        assert!(matches!(
            bad.validate(&base()),
            Err(LayoutError::ObstacleHeight(_))
        ));
    }

    #[test]
    fn blocking_obstacles_leave_movement_and_give_cover_beside_them() {
        let grid = base().with_layout(&layout());
        assert!(!grid.is_walkable(Cell::new(3, 2)));
        assert_eq!(grid.cover_at(Cell::new(3, 2)), Cover::Half);
        assert!(grid.is_difficult(Cell::new(2, 2)));
        let (from, cover) = grid.cover_beside(Cell::new(2, 3)).unwrap();
        assert_eq!((from, cover), (Cell::new(3, 2), Cover::Half));
        assert_eq!(grid.cover_beside(Cell::new(6, 5)), None);
    }

    #[test]
    fn the_strongest_adjacent_cover_wins_and_a_low_obstacle_cannot_be_stood_on() {
        let mut layout = layout();
        layout.obstacles[0].cover = Cover::Total;
        layout.obstacles[1].blocks_movement = false;
        let grid = base().with_layout(&layout);
        let (cell, cover) = grid.cover_beside(Cell::new(3, 3)).unwrap();
        assert_eq!((cell, cover), (Cell::new(3, 2), Cover::Total));
        assert!(grid.is_walkable(Cell::new(4, 3)), "crossable");
        assert!(
            !grid.is_standable(Cell::new(4, 3)),
            "but not a place to stop"
        );
        let beside = grid
            .free_cell_beside(Cell::new(5, 3), Cell::new(1, 5), &[])
            .unwrap();
        assert!(grid.is_standable(beside));
    }

    #[test]
    fn a_path_goes_around_a_crate_and_never_through_it() {
        let grid = base().with_layout(&layout());
        let path = grid.path(Cell::new(3, 5), Cell::new(3, 0)).unwrap();
        assert!(path.iter().all(|cell| grid.is_walkable(*cell)));
        assert!(!path.contains(&Cell::new(3, 2)));
        for pair in path.windows(2) {
            assert_eq!(pair[0].steps_to(pair[1]), 1);
        }
    }

    #[test]
    fn the_path_is_deterministic_and_prefers_straight_runs_over_staircases() {
        let grid = Grid::open_except([0.0, 0.0], 1.0, 8, 8, &[]).unwrap();
        let first = grid.path(Cell::new(0, 0), Cell::new(6, 2)).unwrap();
        assert_eq!(first, grid.path(Cell::new(0, 0), Cell::new(6, 2)).unwrap());
        assert_eq!(first.len(), 7, "six moves, the shortest in steps");
    }

    #[test]
    fn choosing_is_stable_for_a_key_and_spreads_over_the_layouts() {
        let layouts: Vec<Layout> = (0..3)
            .map(|index| Layout {
                id: format!("layout-{index}"),
                ..layout()
            })
            .collect();
        let first = choose(&layouts, "scene-salt-quay-arrival|a|b")
            .unwrap()
            .id
            .clone();
        for _ in 0..5 {
            assert_eq!(
                choose(&layouts, "scene-salt-quay-arrival|a|b").unwrap().id,
                first
            );
        }
        let picked: BTreeSet<String> = (0..60)
            .map(|index| {
                choose(&layouts, &format!("fight-{index}"))
                    .unwrap()
                    .id
                    .clone()
            })
            .collect();
        assert_eq!(picked.len(), 3);
        assert!(choose(&[], "x").is_none());
    }
}
