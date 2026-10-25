use super::{RuleError, Size};
use serde::{Deserialize, Serialize, de::IgnoredAny};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Column/row of a five-foot square. Coordinates are zero based.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, Ord, PartialEq, PartialOrd)]
pub struct Cell(pub u16, pub u16);
impl Cell {
    /// SRD Playing on a Grid: every square, including diagonals, counts as five feet.
    pub fn distance(self, other: Self) -> u32 {
        u32::from(self.0.abs_diff(other.0).max(self.1.abs_diff(other.1))) * 5
    }
}

/// Whether the fight is in sunlight. SRD 5.2.1 p302 Kobold Warrior: while in sunlight it has
/// Disadvantage on ability checks and attack rolls. The painted scenes are night or interior;
/// the photographic scan scenes are daylight. The fact is typed and carried by the map, never
/// inferred from a scene name inside the rules.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum Lighting {
    Sunlight,
    NightOrInterior,
    Darkness,
}

/// Authored obstacle coverage, interpreted by the disclosed centre-line convention.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum Cover {
    None,
    Half,
    ThreeQuarters,
    Total,
}
impl Cover {
    /// SRD Cover: both AC and Dexterity saving throw bonus; total forbids direct targeting.
    pub fn bonus(self) -> i16 {
        match self {
            Self::Half => 2,
            Self::ThreeQuarters => 5,
            Self::None | Self::Total => 0,
        }
    }
}

/// Deliberate parser subset: flattened fields are consumed as IgnoredAny, never mechanics.
#[derive(Debug, Deserialize)]
pub struct BattleSceneData {
    pub schema: u16,
    pub grid: GridData,
    pub layouts: Vec<LayoutData>,
    #[serde(flatten)]
    pub rendering_fields: BTreeMap<String, IgnoredAny>,
}
#[derive(Debug, Deserialize)]
pub struct GridData {
    pub cols: u16,
    pub rows: u16,
    pub blocked: Vec<Cell>,
    #[serde(default)]
    pub blocked_steps: Vec<[Cell; 2]>,
    #[serde(default)]
    pub step_heights_dm: Option<Vec<u16>>,
    #[serde(flatten)]
    pub rendering_fields: BTreeMap<String, IgnoredAny>,
}
#[derive(Debug, Deserialize)]
pub struct LayoutData {
    pub id: String,
    pub obstacles: Vec<ObstacleData>,
    pub difficult: Vec<Cell>,
    pub spawns: SpawnData,
    #[serde(flatten)]
    pub rendering_fields: BTreeMap<String, IgnoredAny>,
}
#[derive(Debug, Deserialize)]
pub struct ObstacleData {
    pub id: String,
    pub kind: String,
    pub cells: Vec<Cell>,
    pub blocks_movement: bool,
    pub cover: Cover,
    pub height_m: f64,
    #[serde(flatten)]
    pub rendering_fields: BTreeMap<String, IgnoredAny>,
}
#[derive(Debug, Deserialize)]
pub struct SpawnData {
    pub heroes: Vec<Cell>,
    pub enemies: Vec<Cell>,
    #[serde(flatten)]
    pub rendering_fields: BTreeMap<String, IgnoredAny>,
}

/// Validated mechanics map. Rendering scale never changes the five-foot rules grid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BattleMap {
    id: String,
    cols: u16,
    rows: u16,
    blocked: BTreeSet<Cell>,
    blocked_steps: BTreeSet<[Cell; 2]>,
    step_heights: BTreeMap<[Cell; 2], Option<u16>>,
    difficult: BTreeSet<Cell>,
    cover: BTreeMap<Cell, Cover>,
    cover_heights_dm: BTreeMap<Cell, u16>,
    hero_spawns: Vec<Cell>,
    enemy_spawns: Vec<Cell>,
    lighting: Lighting,
}
impl BattleMap {
    /// Parse supplied bytes without I/O; require schema v2 and validate every layout.
    pub fn from_json(bytes: &[u8], layout: &str) -> Result<Self, RuleError> {
        if bytes.len() > 1024 * 1024 {
            return Err(RuleError::Capacity);
        }
        let data: BattleSceneData =
            serde_json::from_slice(bytes).map_err(|_| RuleError::MapData)?;
        Self::from_data(&data, layout)
    }

    pub fn from_data(data: &BattleSceneData, selected: &str) -> Result<Self, RuleError> {
        if data.schema != 2 || data.grid.cols == 0 || data.grid.rows == 0 {
            return Err(RuleError::MapData);
        }
        if data.grid.cols > 128 || data.grid.rows > 128 || data.layouts.len() > 32 {
            return Err(RuleError::Capacity);
        }
        let cell_capacity = usize::from(data.grid.cols) * usize::from(data.grid.rows);
        if data.grid.blocked.len() > cell_capacity
            || data.grid.blocked_steps.len() > cell_capacity * 4
        {
            return Err(RuleError::Capacity);
        }
        if data
            .grid
            .step_heights_dm
            .as_ref()
            .is_some_and(|heights| heights.len() != data.grid.blocked_steps.len())
        {
            return Err(RuleError::MapData);
        }
        let mut ids = BTreeSet::new();
        let mut result = None;
        for layout in &data.layouts {
            if !valid_id(&layout.id) || !ids.insert(&layout.id) {
                return Err(RuleError::DuplicateId);
            }
            if layout.obstacles.len() > 1024 {
                return Err(RuleError::Capacity);
            }
            let obstacle_cells = layout
                .obstacles
                .iter()
                .try_fold(0_usize, |sum, obstacle| {
                    sum.checked_add(obstacle.cells.len())
                })
                .ok_or(RuleError::Capacity)?;
            if obstacle_cells > cell_capacity * 2
                || layout.difficult.len() > cell_capacity
                || layout.spawns.heroes.len() > 64
                || layout.spawns.enemies.len() > 64
                || layout
                    .spawns
                    .heroes
                    .len()
                    .saturating_add(layout.spawns.enemies.len())
                    > 64
            {
                return Err(RuleError::Capacity);
            }
            let mut map = Self {
                id: layout.id.clone(),
                cols: data.grid.cols,
                rows: data.grid.rows,
                blocked: BTreeSet::new(),
                blocked_steps: BTreeSet::new(),
                step_heights: BTreeMap::new(),
                difficult: BTreeSet::new(),
                cover: BTreeMap::new(),
                cover_heights_dm: BTreeMap::new(),
                hero_spawns: layout.spawns.heroes.clone(),
                enemy_spawns: layout.spawns.enemies.clone(),
                lighting: Lighting::NightOrInterior,
            };
            for &cell in &data.grid.blocked {
                map.require_cell(cell)?;
                if !map.blocked.insert(cell) {
                    return Err(RuleError::DuplicateCell);
                }
            }
            let mut obstacle_ids = BTreeSet::new();
            for (index, &[from, to]) in data.grid.blocked_steps.iter().enumerate() {
                map.require_cell(from)?;
                map.require_cell(to)?;
                if from.distance(to) != 5 {
                    return Err(RuleError::MapData);
                }
                let step = ordered_step(from, to);
                if !map.blocked_steps.insert(step) {
                    return Err(RuleError::DuplicateCell);
                }
                let height = data
                    .grid
                    .step_heights_dm
                    .as_ref()
                    .map(|heights| heights[index]);
                map.step_heights.insert(step, height);
            }
            for obstacle in &layout.obstacles {
                if !valid_id(&obstacle.id) || !obstacle_ids.insert(&obstacle.id) {
                    return Err(RuleError::DuplicateId);
                }
                if !valid_id(&obstacle.kind)
                    || obstacle.cells.is_empty()
                    || !obstacle.height_m.is_finite()
                    || obstacle.height_m < 0.0
                {
                    return Err(RuleError::MapData);
                }
                let height_dm = (obstacle.height_m * 10.0).round();
                if height_dm > f64::from(u16::MAX) {
                    return Err(RuleError::MapData);
                }
                let height_dm = height_dm as u16;
                let mut cells = BTreeSet::new();
                for &cell in &obstacle.cells {
                    map.require_cell(cell)?;
                    if !cells.insert(cell) {
                        return Err(RuleError::DuplicateCell);
                    }
                    if obstacle.blocks_movement {
                        map.blocked.insert(cell);
                    }
                    map.cover
                        .entry(cell)
                        .and_modify(|cover| *cover = (*cover).max(obstacle.cover))
                        .or_insert(obstacle.cover);
                    if obstacle.cover != Cover::None {
                        map.cover_heights_dm
                            .entry(cell)
                            .and_modify(|height| *height = (*height).max(height_dm))
                            .or_insert(height_dm);
                    }
                }
            }
            for &cell in &layout.difficult {
                map.require_cell(cell)?;
                if !map.difficult.insert(cell) {
                    return Err(RuleError::DuplicateCell);
                }
            }
            let mut spawns = BTreeSet::new();
            for &cell in layout.spawns.heroes.iter().chain(&layout.spawns.enemies) {
                map.require_cell(cell)?;
                if map.is_blocked(cell) || !spawns.insert(cell) {
                    return Err(RuleError::Spawn);
                }
            }
            if layout.spawns.heroes.is_empty()
                || layout.spawns.enemies.is_empty()
                || spawns.len() > 64
            {
                return Err(RuleError::Spawn);
            }
            if let Some(&start) = spawns.first() {
                let mut reached = BTreeSet::from([start]);
                let mut queue = VecDeque::from([start]);
                while let Some(cell) = queue.pop_front() {
                    for next in map.neighbors(cell) {
                        if map.step_clear(cell, next) && reached.insert(next) {
                            queue.push_back(next);
                        }
                    }
                }
                if !spawns.is_subset(&reached) {
                    return Err(RuleError::UnreachableSpawn);
                }
            }
            if layout.id == selected {
                result = Some(map);
            }
        }
        result.ok_or(RuleError::UnknownLayout)
    }
    /// Parsed scene data carries no lighting fact (the renderer's scene files reject unknown
    /// fields), so the staging caller states it. Authored fixtures default to night/interior.
    pub fn with_lighting(mut self, lighting: Lighting) -> Self {
        self.lighting = lighting;
        self
    }
    pub fn lighting(&self) -> Lighting {
        self.lighting
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn dimensions(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }
    pub fn hero_spawns(&self) -> &[Cell] {
        &self.hero_spawns
    }
    pub fn enemy_spawns(&self) -> &[Cell] {
        &self.enemy_spawns
    }
    pub fn contains(&self, cell: Cell) -> bool {
        cell.0 < self.cols && cell.1 < self.rows
    }
    pub fn is_blocked(&self, cell: Cell) -> bool {
        !self.contains(cell) || self.blocked.contains(&cell)
    }
    pub fn is_difficult(&self, cell: Cell) -> bool {
        self.difficult.contains(&cell)
    }
    fn require_cell(&self, cell: Cell) -> Result<(), RuleError> {
        if self.contains(cell) {
            Ok(())
        } else {
            Err(RuleError::OutOfBounds)
        }
    }
    pub(crate) fn neighbors(&self, cell: Cell) -> Vec<Cell> {
        let mut cells = Vec::with_capacity(8);
        for col in cell.0.saturating_sub(1)..=cell.0.saturating_add(1).min(self.cols - 1) {
            for row in cell.1.saturating_sub(1)..=cell.1.saturating_add(1).min(self.rows - 1) {
                let next = Cell(col, row);
                if next != cell {
                    cells.push(next);
                }
            }
        }
        cells
    }
    /// SRD: no diagonal movement past a wall or terrain feature filling its corner.
    pub(crate) fn step_clear(&self, from: Cell, to: Cell) -> bool {
        self.contains(from)
            && self.contains(to)
            && from.distance(to) == 5
            && !self.blocked_steps.contains(&ordered_step(from, to))
            && !self.is_blocked(to)
            && (from.0 == to.0
                || from.1 == to.1
                || (!self.is_blocked(Cell(from.0, to.1)) && !self.is_blocked(Cell(to.0, from.1))))
    }
    /// Disclosed geometry convention: centre-to-centre open-interior intersection.
    /// Exact tangency at a corner does not intersect; strongest cover applies, never sums.
    pub fn cover_between(
        &self,
        from: Cell,
        to: Cell,
        creatures: &[Cell],
    ) -> Result<Cover, RuleError> {
        self.cover_between_for_size(from, to, creatures, Size::Medium)
    }
    pub(crate) fn cover_between_for_size(
        &self,
        from: Cell,
        to: Cell,
        creatures: &[Cell],
        target_size: Size,
    ) -> Result<Cover, RuleError> {
        if creatures.len() > 64 * 9 {
            return Err(RuleError::Capacity);
        }
        self.require_cell(from)?;
        self.require_cell(to)?;
        let mut cover = Cover::None;
        cover = cover.max(barrier_between(
            &self.step_heights,
            from,
            to,
            target_height_dm(target_size),
        ));
        for (&cell, &grade) in &self.cover {
            if cell != from && intersects(from, to, cell) {
                // SRD 5.2.1 p15: a barrier's height determines whether it covers
                // half, three-quarters, or the whole target. The authored cover
                // label remains a bounded legacy hint, but cannot turn a low
                // obstacle into Total Cover.
                let height_grade = self
                    .cover_heights_dm
                    .get(&cell)
                    .copied()
                    .map_or(grade, |height| {
                        grade.min(barrier_cover(height, target_height_dm(target_size)))
                    });
                cover = cover.max(height_grade);
            }
        }
        for &cell in creatures {
            self.require_cell(cell)?;
            if cell != from && cell != to && intersects(from, to, cell) {
                cover = cover.max(Cover::Half);
            }
        }
        Ok(cover)
    }
}

fn ordered_step(from: Cell, to: Cell) -> [Cell; 2] {
    if from < to { [from, to] } else { [to, from] }
}

// A blocked step between two cells that share an edge is a barrier along that edge (a fence
// runs there). In doubled grid coordinates a cell's centre is odd and an edge's ends are even,
// so a line between two centres never runs along a barrier. It is stopped when it passes
// through the inside of a barrier, or through a corner where barriers lie on both sides of it
// (the wall goes on past the corner). A line that only touches the end of a wall is not
// stopped, the same way a line that only touches an obstacle's corner takes no cover from it.
// A blocked diagonal step is a body-width fact (a figure cannot squeeze round that corner) and
// stops no line. All arithmetic is integral.
pub(crate) fn target_height_dm(size: Size) -> u16 {
    // The scene rules have no creature-height field; use representative heights for its sizes.
    match size {
        Size::Small => 12,
        Size::Medium => 18,
        Size::Large => 30,
        Size::Huge => 45,
    }
}

fn barrier_cover(height_dm: u16, target_height_dm: u16) -> Cover {
    let height = u32::from(height_dm);
    let target = u32::from(target_height_dm);
    // SRD 5.2.1 p15: Half Cover covers at least half the target, Three-Quarters
    // at least three-quarters, and Total Cover the whole target.
    if height * 2 < target {
        Cover::None
    } else if height * 4 < target * 3 {
        Cover::Half
    } else if height < target {
        Cover::ThreeQuarters
    } else {
        Cover::Total
    }
}

fn barrier_between(
    steps: &BTreeMap<[Cell; 2], Option<u16>>,
    from: Cell,
    to: Cell,
    target_height_dm: u16,
) -> Cover {
    if from == to {
        return Cover::None;
    }
    let line_start = [i64::from(from.0) * 2 + 1, i64::from(from.1) * 2 + 1];
    let line_end = [i64::from(to.0) * 2 + 1, i64::from(to.1) * 2 + 1];
    let signed_area = |origin: [i64; 2], edge_end: [i64; 2], point: [i64; 2]| {
        (edge_end[0] - origin[0]) * (point[1] - origin[1])
            - (edge_end[1] - origin[1]) * (point[0] - origin[0])
    };
    let within_line_bounds = |point: [i64; 2]| {
        (0..2).all(|axis| {
            line_start[axis].min(line_end[axis]) <= point[axis]
                && point[axis] <= line_start[axis].max(line_end[axis])
        })
    };
    // Corners the line touches, with the sides of the line a barrier leaves them on.
    let mut cover = Cover::None;
    let mut touched: Vec<([i64; 2], Option<Cover>, Option<Cover>)> = Vec::new();
    for (&[a, b], &height_dm) in steps {
        let grade = height_dm.map_or(Cover::Total, |height_dm| {
            barrier_cover(height_dm, target_height_dm)
        });
        let delta = [
            i64::from(b.0) - i64::from(a.0),
            i64::from(b.1) - i64::from(a.1),
        ];
        if delta[0] != 0 && delta[1] != 0 {
            continue;
        }
        let mid = [
            i64::from(a.0) + i64::from(b.0) + 1,
            i64::from(a.1) + i64::from(b.1) + 1,
        ];
        let side_point_r = [mid[0] - delta[1], mid[1] + delta[0]];
        let side_point_s = [mid[0] + delta[1], mid[1] - delta[0]];
        let (side_r, side_s) = (
            signed_area(line_start, line_end, side_point_r).signum(),
            signed_area(line_start, line_end, side_point_s).signum(),
        );
        if side_r * side_s < 0 {
            if signed_area(side_point_r, side_point_s, line_start).signum()
                * signed_area(side_point_r, side_point_s, line_end).signum()
                < 0
            {
                cover = cover.max(grade);
            }
            continue;
        }
        for (corner, side) in [(side_point_r, side_s), (side_point_s, side_r)] {
            if signed_area(line_start, line_end, corner) != 0 || !within_line_bounds(corner) {
                continue;
            }
            let index = match touched.iter().position(|(known, _, _)| *known == corner) {
                Some(index) => index,
                None => {
                    touched.push((corner, None, None));
                    touched.len() - 1
                }
            };
            let slot = if side > 0 {
                &mut touched[index].1
            } else if side < 0 {
                &mut touched[index].2
            } else {
                continue;
            };
            *slot = Some(slot.map_or(grade, |existing| existing.max(grade)));
        }
    }
    for (_, left, right) in touched {
        if let (Some(left), Some(right)) = (left, right) {
            cover = cover.max(left).max(right);
        }
    }
    cover
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
// Rational clipping: cell centres are odd coordinates, boundaries even. Avoid floating point
// in rules decisions. The admitted 128-square extent bounds every cross product below.
fn intersects(from: Cell, to: Cell, cell: Cell) -> bool {
    let origin = [i64::from(from.0) * 2 + 1, i64::from(from.1) * 2 + 1];
    let end = [i64::from(to.0) * 2 + 1, i64::from(to.1) * 2 + 1];
    let min = [i64::from(cell.0) * 2, i64::from(cell.1) * 2];
    let mut low = (0_i64, 1_i64);
    let mut high = (1_i64, 1_i64);
    for axis in 0..2 {
        let delta = end[axis] - origin[axis];
        if delta == 0 {
            if origin[axis] <= min[axis] || origin[axis] >= min[axis] + 2 {
                return false;
            }
        } else {
            let mut first = (min[axis] - origin[axis], delta);
            let mut last = (min[axis] + 2 - origin[axis], delta);
            if delta < 0 {
                first = (-last.0, -last.1);
                last = (origin[axis] - min[axis], -delta);
            }
            if first.0 * low.1 > low.0 * first.1 {
                low = first;
            }
            if last.0 * high.1 < high.0 * last.1 {
                high = last;
            }
        }
    }
    low.0 * high.1 < high.0 * low.1
}

#[cfg(test)]
mod collision_tests {
    use super::*;

    #[test]
    fn scan_barriers_refuse_steps_reach_and_line_of_effect_in_both_directions() {
        let bytes = br#"{"schema":2,"grid":{"cols":4,"rows":4,"blocked":[],"blocked_steps":[[[1,1],[2,1]]]},"layouts":[{"id":"scan","obstacles":[],"difficult":[],"spawns":{"heroes":[[0,0]],"enemies":[[0,3]]}}]}"#;
        let map = BattleMap::from_json(bytes, "scan").unwrap();
        assert!(!map.step_clear(Cell(1, 1), Cell(2, 1)));
        assert!(!map.step_clear(Cell(2, 1), Cell(1, 1)));
        assert_eq!(
            map.cover_between(Cell(0, 1), Cell(3, 1), &[]),
            Ok(Cover::Total)
        );
        assert_eq!(
            map.cover_between(Cell(3, 1), Cell(0, 1), &[]),
            Ok(Cover::Total)
        );
        assert_eq!(
            map.cover_between(Cell(0, 0), Cell(3, 0), &[]),
            Ok(Cover::None)
        );
        assert_eq!(
            map.cover_between(Cell(0, 1), Cell(3, 1), &[Cell(99, 99)]),
            Err(RuleError::OutOfBounds)
        );
    }
    fn scan(steps: &str) -> BattleMap {
        let bytes = format!(
            r#"{{"schema":2,"grid":{{"cols":6,"rows":6,"blocked":[],"blocked_steps":{steps}}},"layouts":[{{"id":"scan","obstacles":[],"difficult":[],"spawns":{{"heroes":[[0,0]],"enemies":[[5,5]]}}}}]}}"#
        );
        BattleMap::from_json(bytes.as_bytes(), "scan").unwrap()
    }

    #[test]
    fn a_blocked_diagonal_step_stops_a_body_and_no_line() {
        let map = scan("[[[1,1],[2,2]]]");
        assert!(!map.step_clear(Cell(1, 1), Cell(2, 2)));
        for (from, to) in [
            (Cell(1, 1), Cell(2, 2)),
            (Cell(0, 2), Cell(1, 2)),
            (Cell(2, 1), Cell(3, 1)),
            (Cell(0, 0), Cell(4, 4)),
        ] {
            assert_eq!(map.cover_between(from, to, &[]), Ok(Cover::None));
        }
    }

    #[test]
    fn a_line_past_the_end_of_a_wall_is_clear_and_a_line_through_its_joint_or_corner_is_not() {
        // A wall along the top edge of row 2, under columns 0 to 2: it ends at the corner
        // shared by cells (2,1), (3,1), (2,2) and (3,2).
        let wall = scan("[[[0,1],[0,2]],[[1,1],[1,2]],[[2,1],[2,2]]]");
        // Through the inside of a piece, and through the joint between two pieces.
        assert_eq!(
            wall.cover_between(Cell(1, 3), Cell(1, 0), &[]),
            Ok(Cover::Total)
        );
        assert_eq!(
            wall.cover_between(Cell(0, 2), Cell(1, 1), &[]),
            Ok(Cover::Total)
        );
        assert_eq!(
            wall.cover_between(Cell(1, 1), Cell(0, 2), &[]),
            Ok(Cover::Total)
        );
        // Past the wall's end, touching its last corner: the weasel in the open can be shot.
        assert_eq!(
            wall.cover_between(Cell(2, 2), Cell(3, 1), &[]),
            Ok(Cover::None)
        );
        assert_eq!(
            wall.cover_between(Cell(1, 3), Cell(4, 0), &[]),
            Ok(Cover::None)
        );
        assert_eq!(
            wall.cover_between(Cell(4, 0), Cell(1, 3), &[]),
            Ok(Cover::None)
        );
        // Clear of it altogether.
        assert_eq!(
            wall.cover_between(Cell(3, 3), Cell(3, 0), &[]),
            Ok(Cover::None)
        );
        // The same wall turning a corner downward at its end: the line through that corner now
        // has wall on both sides of it and is stopped; a line outside the corner is not.
        let corner = scan("[[[0,1],[0,2]],[[1,1],[1,2]],[[2,1],[2,2]],[[2,2],[3,2]]]");
        assert_eq!(
            corner.cover_between(Cell(2, 2), Cell(3, 1), &[]),
            Ok(Cover::Total)
        );
        assert_eq!(
            corner.cover_between(Cell(3, 2), Cell(2, 1), &[]),
            Ok(Cover::None)
        );
    }

    #[test]
    fn scan_barrier_heights_grade_cover_against_medium_height_srd_p15() {
        let map_at = |height_dm: Option<u16>| {
            let mut data = serde_json::json!({
                "schema": 2,
                "grid": {
                    "cols": 4,
                    "rows": 4,
                    "blocked": [],
                    "blocked_steps": [[[1, 1], [2, 1]]]
                },
                "layouts": [{
                    "id": "scan",
                    "obstacles": [],
                    "difficult": [],
                    "spawns": {"heroes": [[0, 0]], "enemies": [[0, 3]]}
                }]
            });
            if let Some(height_dm) = height_dm {
                data["grid"]["step_heights_dm"] = serde_json::json!([height_dm]);
            }
            BattleMap::from_json(data.to_string().as_bytes(), "scan").unwrap()
        };

        for (height_dm, expected) in [
            (Some(4), Cover::None),
            (Some(5), Cover::None),
            (Some(9), Cover::Half),
            (Some(10), Cover::Half),
            (Some(13), Cover::Half),
            (Some(14), Cover::ThreeQuarters),
            (Some(18), Cover::Total),
            (None, Cover::Total),
        ] {
            assert_eq!(
                map_at(height_dm).cover_between(Cell(0, 1), Cell(3, 1), &[]),
                Ok(expected),
                "height {height_dm:?} dm"
            );
        }
    }

    #[test]
    fn scan_barrier_cover_scales_to_the_target_size_srd_p15() {
        let map = serde_json::from_value::<BattleSceneData>(serde_json::json!({
            "schema": 2,
            "grid": {
                "cols": 4,
                "rows": 4,
                "blocked": [],
                "blocked_steps": [[[1, 1], [2, 1]]],
                "step_heights_dm": [4]
            },
            "layouts": [{
                "id": "scan",
                "obstacles": [],
                "difficult": [],
                "spawns": {"heroes": [[0, 0]], "enemies": [[0, 3]]}
            }]
        }))
        .unwrap();
        let map = BattleMap::from_data(&map, "scan").unwrap();
        assert_eq!(
            map.cover_between_for_size(Cell(0, 1), Cell(3, 1), &[], Size::Medium),
            Ok(Cover::None)
        );
        assert_eq!(
            map.cover_between_for_size(Cell(0, 1), Cell(3, 1), &[], Size::Small),
            Ok(Cover::None)
        );
    }

    #[test]
    fn obstacle_cover_uses_height_against_target_size_srd_521_p15() {
        let map = |height_m: f64| {
            let data = serde_json::json!({
                "schema": 2,
                "grid": {"cols": 5, "rows": 1, "blocked": []},
                "layouts": [{
                    "id": "height",
                    "obstacles": [{
                        "id": "bush",
                        "kind": "bush",
                        "cells": [[2, 0]],
                        "blocks_movement": false,
                        "cover": "total",
                        "height_m": height_m
                    }],
                    "difficult": [],
                    "spawns": {"heroes": [[0, 0]], "enemies": [[4, 0]]}
                }]
            });
            BattleMap::from_json(data.to_string().as_bytes(), "height").unwrap()
        };
        assert_eq!(
            map(0.4).cover_between(Cell(0, 0), Cell(4, 0), &[]),
            Ok(Cover::None)
        );
        assert_eq!(
            map(1.0).cover_between(Cell(0, 0), Cell(4, 0), &[]),
            Ok(Cover::Half)
        );
        assert_eq!(
            map(2.0).cover_between(Cell(0, 0), Cell(4, 0), &[]),
            Ok(Cover::Total)
        );
    }
}
