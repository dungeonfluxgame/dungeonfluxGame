use super::RuleError;
use serde::{Deserialize, de::IgnoredAny};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Column/row of a five-foot square. Coordinates are zero based.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
pub struct Cell(pub u16, pub u16);
impl Cell {
    /// SRD Playing on a Grid: every square, including diagonals, counts as five feet.
    pub fn distance(self, other: Self) -> u32 {
        u32::from(self.0.abs_diff(other.0).max(self.1.abs_diff(other.1))) * 5
    }
}

/// Authored obstacle coverage, interpreted by the disclosed centre-line convention.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
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
    difficult: BTreeSet<Cell>,
    cover: BTreeMap<Cell, Cover>,
    hero_spawns: Vec<Cell>,
    enemy_spawns: Vec<Cell>,
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
        if data.grid.blocked.len() > cell_capacity {
            return Err(RuleError::Capacity);
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
                difficult: BTreeSet::new(),
                cover: BTreeMap::new(),
                hero_spawns: layout.spawns.heroes.clone(),
                enemy_spawns: layout.spawns.enemies.clone(),
            };
            for &cell in &data.grid.blocked {
                map.require_cell(cell)?;
                if !map.blocked.insert(cell) {
                    return Err(RuleError::DuplicateCell);
                }
            }
            let mut obstacle_ids = BTreeSet::new();
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
        if creatures.len() > 64 {
            return Err(RuleError::Capacity);
        }
        self.require_cell(from)?;
        self.require_cell(to)?;
        let mut cover = Cover::None;
        for (&cell, &grade) in &self.cover {
            if cell != from && intersects(from, to, cell) {
                cover = cover.max(grade);
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
