//! Offline scan-grid authoring. No providers or runtime. Usage:
//! splat_collision <scene.json> <scan/v1> <level> <versioned-output.json>
//! WebP decoding uses installed Python/Pillow, streaming one chunk at a time through stdout.
//! Only cells, blocked steps, ground heights and repaired layouts enter scene JSON; no voxels.

#[cfg(target_arch = "wasm32")]
fn main() {}
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), String> {
    native::run()
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use df_render::battle::collider::VoxelBuilder;
    use df_render::battle::{
        Cell, Grid, HeightField, SogLoader, SogMeta, SogPlanes, SplatCloud, SplatLoader,
        SplatTransform, Vec3, parse_scene,
    };
    use serde_json::{Value, json};
    use std::{
        collections::{BTreeMap, BTreeSet, VecDeque},
        path::{Path, PathBuf},
        process::Command,
    };

    fn read(path: &Path) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
    }
    fn number(value: &Value) -> Result<f32, String> {
        value
            .as_f64()
            .map(|v| v as f32)
            .ok_or("scan bound number".into())
    }
    fn vector(value: &Value) -> Result<Vec3, String> {
        Ok(Vec3::new(
            number(&value[0])?,
            number(&value[1])?,
            number(&value[2])?,
        ))
    }
    fn ranges(
        node: &Value,
        transform: SplatTransform,
        low: Vec3,
        high: Vec3,
        level: &str,
        selected: &mut BTreeMap<u32, Vec<(usize, usize)>>,
    ) -> Result<(), String> {
        let a = vector(&node["bound"]["min"])?;
        let b = vector(&node["bound"]["max"])?;
        let mut min = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
        let mut max = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        for x in [a.x, b.x] {
            for y in [a.y, b.y] {
                for z in [a.z, b.z] {
                    let p = transform.to_world(Vec3::new(x, y, z));
                    min = Vec3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
                    max = Vec3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
                }
            }
        }
        if max.x < low.x
            || min.x > high.x
            || max.y < low.y
            || min.y > high.y
            || max.z < low.z
            || min.z > high.z
        {
            return Ok(());
        }
        if let Some(children) = node["children"].as_array().filter(|v| !v.is_empty()) {
            for child in children {
                ranges(child, transform, low, high, level, selected)?;
            }
        } else {
            let range = &node["lods"][level];
            let file = range["file"].as_u64().ok_or("missing selected level")? as u32;
            let offset = range["offset"].as_u64().ok_or("range offset")? as usize;
            let count = range["count"].as_u64().ok_or("range count")? as usize;
            selected.entry(file).or_default().push((offset, count));
        }
        Ok(())
    }
    fn plane(path: &Path) -> Result<Vec<u8>, String> {
        let output = Command::new("python").args(["-c",
            "from PIL import Image; import sys; im=Image.open(sys.argv[1]); assert im.width*im.height<=1048576; sys.stdout.buffer.write(im.convert('RGBA').tobytes())"])
            .arg(path).output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "{}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(output.stdout)
    }
    fn chunk(folder: &Path) -> Result<Vec<df_render::battle::RawSplat>, String> {
        let meta =
            SogMeta::parse(&read(&folder.join("meta.json"))?).map_err(|e| format!("{e:?}"))?;
        let means_l = plane(&folder.join(&meta.means_files[0]))?;
        let means_u = plane(&folder.join(&meta.means_files[1]))?;
        let quats = plane(&folder.join(&meta.quats_file))?;
        let scales = plane(&folder.join(&meta.scales_file))?;
        let sh0 = plane(&folder.join(&meta.sh0_file))?;
        SogLoader {
            meta: &meta,
            planes: SogPlanes {
                means_l: &means_l,
                means_u: &means_u,
                quats: &quats,
                scales: &scales,
                sh0: &sh0,
            },
        }
        .load()
        .map_err(|e| format!("{e:?}"))
    }
    fn ascii(value: &Value, grid: &Grid) {
        for layout in value["layouts"].as_array().into_iter().flatten() {
            let obstacles: BTreeSet<_> = layout["obstacles"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|v| v["blocks_movement"] == true)
                .flat_map(|v| v["cells"].as_array().into_iter().flatten())
                .map(|v| Cell::new(v[0].as_i64().unwrap() as i32, v[1].as_i64().unwrap() as i32))
                .collect();
            let blocked = |c| !grid.is_walkable(c) || obstacles.contains(&c);
            let step_clear = |a, b| !blocked(a) && !blocked(b) && grid.step_clear(a, b);
            println!(
                "{} / {} (H hero, E enemy, # blocked, | - / \\ blocked steps)",
                value["location_id"], layout["id"]
            );
            let cells = |key: &str| -> BTreeSet<Cell> {
                layout["spawns"][key]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| {
                        Cell::new(v[0].as_i64().unwrap() as i32, v[1].as_i64().unwrap() as i32)
                    })
                    .collect()
            };
            let heroes = cells("heroes");
            let enemies = cells("enemies");
            for row in 0..grid.rows() as i32 {
                let mut line = String::new();
                let mut below = String::new();
                for col in 0..grid.cols() as i32 {
                    let cell = Cell::new(col, row);
                    line.push(if blocked(cell) {
                        '#'
                    } else if heroes.contains(&cell) {
                        'H'
                    } else if enemies.contains(&cell) {
                        'E'
                    } else {
                        '.'
                    });
                    line.push(
                        if col + 1 < grid.cols() as i32
                            && !step_clear(cell, Cell::new(col + 1, row))
                        {
                            '|'
                        } else {
                            ' '
                        },
                    );
                    below.push(
                        if row + 1 < grid.rows() as i32
                            && !step_clear(cell, Cell::new(col, row + 1))
                        {
                            '-'
                        } else {
                            ' '
                        },
                    );
                    below.push(
                        if row + 1 < grid.rows() as i32
                            && col + 1 < grid.cols() as i32
                            && (!step_clear(cell, Cell::new(col + 1, row + 1))
                                || !step_clear(Cell::new(col + 1, row), Cell::new(col, row + 1)))
                        {
                            'X'
                        } else {
                            ' '
                        },
                    );
                }
                println!("{line}");
                if row + 1 < grid.rows() as i32 {
                    println!("{below}");
                }
            }
        }
    }

    fn neighbours(
        grid: &Grid,
        free: &BTreeSet<Cell>,
        from: Cell,
        excluded: Option<Cell>,
    ) -> Vec<Cell> {
        let mut result = Vec::new();
        for row_delta in -1..=1 {
            for col_delta in -1..=1 {
                if col_delta == 0 && row_delta == 0 {
                    continue;
                }
                let next = Cell::new(from.col + col_delta, from.row + row_delta);
                if excluded == Some(next) || !free.contains(&next) || !grid.step_clear(from, next) {
                    continue;
                }
                if col_delta != 0
                    && row_delta != 0
                    && (!free.contains(&Cell::new(from.col + col_delta, from.row))
                        || !free.contains(&Cell::new(from.col, from.row + row_delta)))
                {
                    continue;
                }
                result.push(next);
            }
        }
        result
    }

    fn reachable(
        grid: &Grid,
        free: &BTreeSet<Cell>,
        from: Cell,
        excluded: Option<Cell>,
    ) -> BTreeSet<Cell> {
        let mut reached = BTreeSet::new();
        if !free.contains(&from) || excluded == Some(from) {
            return reached;
        }
        let mut pending = vec![from];
        reached.insert(from);
        while let Some(current) = pending.pop() {
            for next in neighbours(grid, free, current, excluded) {
                if reached.insert(next) {
                    pending.push(next);
                }
            }
        }
        reached
    }

    fn largest_area(grid: &Grid, free: &BTreeSet<Cell>, excluded: Option<Cell>) -> usize {
        free.iter()
            .copied()
            .filter(|cell| excluded != Some(*cell))
            .map(|cell| reachable(grid, free, cell, excluded).len())
            .max()
            .unwrap_or(0)
    }

    fn distance(grid: &Grid, free: &BTreeSet<Cell>, from: Cell, to: Cell) -> Option<usize> {
        if !free.contains(&from) || !free.contains(&to) {
            return None;
        }
        let mut reached = BTreeSet::from([from]);
        let mut pending = VecDeque::from([(from, 0)]);
        while let Some((current, current_distance)) = pending.pop_front() {
            let next_distance = current_distance + 1;
            for next in neighbours(grid, free, current, None) {
                if next == to {
                    return Some(next_distance);
                }
                if reached.insert(next) {
                    pending.push_back((next, next_distance));
                }
            }
        }
        None
    }

    fn validate_spawns(
        grid: &Grid,
        free: &BTreeSet<Cell>,
        heroes: &[Cell],
        enemies: &[Cell],
    ) -> Result<(), String> {
        let occupied: BTreeSet<_> = heroes.iter().chain(enemies).copied().collect();
        if occupied.len() != heroes.len() + enemies.len() {
            return Err("duplicate spawn cells".into());
        }
        let largest = largest_area(grid, free, None);
        for side in [heroes, enemies] {
            for &spawn in side {
                if !free.contains(&spawn) {
                    return Err(format!("spawn {spawn:?} is not free ground"));
                }
                if neighbours(grid, free, spawn, None)
                    .iter()
                    .filter(|neighbour| !occupied.contains(neighbour))
                    .count()
                    < 2
                {
                    return Err(format!("spawn {spawn:?} has fewer than two free exits"));
                }
                if reachable(grid, free, spawn, None).len() != largest {
                    return Err(format!("spawn {spawn:?} is outside the largest area"));
                }
                for &other in side.iter().filter(|&&other| other != spawn) {
                    if reachable(grid, free, spawn, Some(other)).len()
                        != largest_area(grid, free, Some(other))
                    {
                        return Err(format!("removing spawn {other:?} isolates {spawn:?}"));
                    }
                }
            }
        }
        for &hero in heroes {
            for &enemy in enemies {
                let steps =
                    distance(grid, free, hero, enemy).ok_or("opposing starts are disconnected")?;
                if steps < 3 {
                    return Err(format!("opposing starts are only {steps} steps apart"));
                }
            }
        }
        Ok(())
    }

    fn largest_connectivity_after_removal(
        grid: &Grid,
        free: &BTreeSet<Cell>,
    ) -> BTreeMap<(Cell, Cell), bool> {
        let mut result = BTreeMap::new();
        for removed in free {
            let mut remaining: BTreeSet<_> = free
                .iter()
                .copied()
                .filter(|cell| cell != removed)
                .collect();
            let mut components = Vec::new();
            while let Some(&start) = remaining.iter().next() {
                let component = reachable(grid, free, start, Some(*removed));
                for cell in &component {
                    remaining.remove(cell);
                }
                components.push(component);
            }
            let largest = components.iter().map(BTreeSet::len).max().unwrap_or(0);
            for component in components {
                let is_largest = component.len() == largest;
                for cell in component {
                    result.insert((cell, *removed), is_largest);
                }
            }
        }
        result
    }

    fn legal_spawn_sets(
        grid: &Grid,
        free: &BTreeSet<Cell>,
        candidates: &[Cell],
        count: usize,
        connected_after_removal: &BTreeMap<(Cell, Cell), bool>,
    ) -> Vec<Vec<Cell>> {
        struct Search<'a> {
            grid: &'a Grid,
            free: &'a BTreeSet<Cell>,
            candidates: &'a [Cell],
            connected_after_removal: &'a BTreeMap<(Cell, Cell), bool>,
        }
        fn collect(
            search: &Search<'_>,
            start: usize,
            remaining: usize,
            selected: &mut Vec<Cell>,
            results: &mut Vec<Vec<Cell>>,
        ) {
            if remaining == 0 {
                let occupied: BTreeSet<_> = selected.iter().copied().collect();
                let valid = selected.iter().all(|&spawn| {
                    neighbours(search.grid, search.free, spawn, None)
                        .iter()
                        .filter(|neighbour| !occupied.contains(neighbour))
                        .count()
                        >= 2
                        && selected
                            .iter()
                            .filter(|&&other| other != spawn)
                            .all(|&other| {
                                search.connected_after_removal.get(&(spawn, other)) == Some(&true)
                            })
                });
                if valid {
                    results.push(selected.clone());
                }
                return;
            }
            if search.candidates.len().saturating_sub(start) < remaining {
                return;
            }
            for index in start..=search.candidates.len() - remaining {
                selected.push(search.candidates[index]);
                collect(search, index + 1, remaining - 1, selected, results);
                selected.pop();
            }
        }

        let mut results = Vec::new();
        let search = Search {
            grid,
            free,
            candidates,
            connected_after_removal,
        };
        collect(&search, 0, count, &mut Vec::new(), &mut results);
        results
    }

    fn assign_nearest_starts(original: &[Cell], selected: &[Cell]) -> Vec<Cell> {
        let mut available = selected.to_vec();
        original
            .iter()
            .map(|old| {
                let index = available
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, cell)| {
                        let dc = cell.col.abs_diff(old.col);
                        let dr = cell.row.abs_diff(old.row);
                        (dc * dc + dr * dr, cell.row, cell.col)
                    })
                    .map(|(index, _)| index)
                    .expect("same number of original and selected starts");
                available.remove(index)
            })
            .collect()
    }

    fn json_cell(value: &Value) -> Result<Cell, String> {
        Ok(Cell::new(
            i32::try_from(value[0].as_i64().ok_or("spawn column")?)
                .map_err(|_| "spawn column bounds")?,
            i32::try_from(value[1].as_i64().ok_or("spawn row")?).map_err(|_| "spawn row bounds")?,
        ))
    }

    fn json_cells(value: &Value) -> Result<Vec<Cell>, String> {
        value
            .as_array()
            .ok_or("spawn cells")?
            .iter()
            .map(json_cell)
            .collect()
    }

    pub fn run() -> Result<(), String> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.len() != 4 {
            return Err("scene.json scan/v1 level output.json required".into());
        }
        let scene_path = PathBuf::from(&args[0]);
        let scan = PathBuf::from(&args[1]);
        let mut value: Value =
            serde_json::from_str(&read(&scene_path)?).map_err(|e| e.to_string())?;
        let scene = parse_scene(&value.to_string()).map_err(|e| format!("{e:?}"))?;
        let layer = scene.splat.as_ref().ok_or("no scan")?;
        let transform = SplatTransform {
            scale: layer.scale,
            offset_y: layer.offset_y,
            rot_y_deg: layer.rot_y_deg,
            translate: layer.translate,
            orient: layer.orient,
        };
        let original = scene.grid.terrain().ok_or("no measured ground hint")?;
        let floor = original
            .heights()
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min)
            - 0.5;
        let top = original
            .heights()
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max)
            + 2.5;
        let origin = Vec3::new(
            original.origin()[0] - 0.5,
            floor,
            original.origin()[1] - 0.5,
        );
        let size = Vec3::new(
            original.cols() as f32 * original.step_m() + 1.0,
            top - floor,
            original.rows() as f32 * original.step_m() + 1.0,
        );
        let dims = [size.x, size.y, size.z].map(|v| (v / 0.1).ceil() as u32);
        let mut builder = VoxelBuilder::new(origin, 0.1, dims).map_err(|e| format!("{e:?}"))?;
        let index: Value =
            serde_json::from_str(&read(&scan.join("lod-meta.json"))?).map_err(|e| e.to_string())?;
        let mut selected = BTreeMap::new();
        // Tree bounds are splat bounds; include 2 metres for ellipsoid support beyond a leaf.
        ranges(
            &index["tree"],
            transform,
            origin - Vec3::new(2.0, 2.0, 2.0),
            origin + size + Vec3::new(2.0, 2.0, 2.0),
            &args[2],
            &mut selected,
        )?;
        println!("BEFORE");
        ascii(&value, &scene.grid);
        let mut total = 0usize;
        for (file, ranges) in selected {
            let filename = index["filenames"][file as usize]
                .as_str()
                .ok_or("chunk filename")?;
            let folder = scan
                .join(filename)
                .parent()
                .ok_or("chunk folder")?
                .to_path_buf();
            let raw = chunk(&folder)?;
            for (offset, count) in ranges {
                let raw = raw.get(offset..offset + count).ok_or("chunk range")?;
                let (cloud, dropped) = SplatCloud::bake(raw, transform);
                if dropped != 0 {
                    return Err(format!("{dropped} invalid splats in {}", folder.display()));
                }
                let centre = scene.grid.centre();
                let visible: Vec<_> = cloud
                    .splats()
                    .iter()
                    .copied()
                    .filter(|splat| {
                        !layer.clip.is_some_and(|(height, radius)| {
                            splat.centre.y > height
                                && (splat.centre.x - centre.x).hypot(splat.centre.z - centre.z)
                                    < radius
                        })
                    })
                    .collect();
                builder
                    .add(&visible)
                    .map_err(|e| format!("{}: {e:?}", folder.display()))?;
                total += raw.len();
            }
            eprintln!("{}: {total} splats processed", folder.display());
        }
        let collider = builder.finish().map_err(|e| format!("{e:?}"))?;
        let mut heights = original.heights().to_vec();
        let mut supported = BTreeSet::new();
        for row in 0..original.rows() {
            for col in 0..original.cols() {
                let x = original.origin()[0] + col as f32 * original.step_m();
                let z = original.origin()[1] + row as f32 * original.step_m();
                let hint = original.height_at(x, z);
                if let Some(y) = collider.ground_height(x, z, hint - 0.35, hint + 1.0) {
                    heights[(row * original.cols() + col) as usize] = y;
                }
                // Outside supported play cells the original surveyed height remains a hint.
            }
        }
        for height in &mut heights {
            *height = (*height * 100.0).round() * 0.01;
        }
        let terrain = HeightField::new(
            original.origin(),
            original.step_m(),
            original.cols(),
            original.rows(),
            heights.clone(),
        )
        .map_err(|e| format!("{e:?}"))?;
        let mut grid = Grid::open_except(
            scene.grid.origin(),
            scene.grid.cell_m(),
            scene.grid.cols(),
            scene.grid.rows(),
            &[],
        )
        .map_err(|e| format!("{e:?}"))?
        .with_terrain(terrain)
        .with_collision(&[])
        .map_err(|e| format!("{e:?}"))?;
        let mut blocked = Vec::new();
        for row in 0..grid.rows() as i32 {
            for col in 0..grid.cols() as i32 {
                let cell = Cell::new(col, row);
                let p = grid.cell_centre(cell);
                let contact = collider
                    .touches_capsule(
                        p + Vec3::new(0.0, 0.6, 0.0),
                        p + Vec3::new(0.0, 1.35, 0.0),
                        0.35,
                    )
                    .map_err(|e| format!("{e:?}"))?;
                if collider
                    .ground_height(p.x, p.z, p.y - 0.35, p.y + 0.2)
                    .is_some()
                {
                    supported.insert(cell);
                }
                println!(
                    "CELL {col},{row} ground {:.3} supported {} contact {contact}",
                    p.y,
                    supported.contains(&cell)
                );
                if !supported.contains(&cell) || contact {
                    blocked.push(cell);
                }
            }
        }
        let field = grid.terrain().ok_or("terrain")?;
        let mut slopes = [0.0f32; 2];
        for row in 0..field.rows() {
            for col in 0..field.cols() {
                let at = (row * field.cols() + col) as usize;
                for (axis, next) in [
                    (0, (col + 1 < field.cols()).then_some(at + 1)),
                    (
                        1,
                        (row + 1 < field.rows()).then_some(at + field.cols() as usize),
                    ),
                ] {
                    if let Some(next) = next {
                        slopes[axis] = slopes[axis].max(
                            (field.heights()[next] - field.heights()[at]).abs() / field.step_m(),
                        );
                    }
                }
            }
        }
        let mut sweep_queries = 0u32;
        let mut steps = Vec::new();
        for row in 0..grid.rows() as i32 {
            for col in 0..grid.cols() as i32 {
                let a = Cell::new(col, row);
                for (dc, dr) in [(1, 0), (-1, 1), (0, 1), (1, 1)] {
                    let b = Cell::new(col + dc, row + dr);
                    if !grid.in_bounds(b) {
                        continue;
                    }
                    let from = grid.cell_centre(a);
                    let to = grid.cell_centre(b);
                    let delta = to - from;
                    let vertical_bound = slopes[0] * delta.x.abs() + slopes[1] * delta.z.abs();
                    // Bilinear terrain gradients bound the full 3D anchor speed, including
                    // rises between equal-height endpoints. Each half interval is <=0.025 m.
                    let count = (delta.x.hypot(delta.z).hypot(vertical_bound) / 0.05)
                        .ceil()
                        .max(1.0) as u32;
                    if count > 2000 {
                        return Err("sweep sampling capacity".into());
                    }
                    let mut collision = blocked.contains(&a) || blocked.contains(&b);
                    for n in 0..=count {
                        if collision {
                            break;
                        }
                        sweep_queries += 1;
                        if sweep_queries > 500_000 {
                            return Err("sweep query capacity".into());
                        }
                        let p = from + (to - from) * (n as f32 / count as f32);
                        let ground = grid.ground_height(p.x, p.z);
                        // Inflating by half a sample interval makes the discrete sweep conservative.
                        collision = collider
                            .touches_capsule(
                                Vec3::new(p.x, ground + 0.6, p.z),
                                Vec3::new(p.x, ground + 1.35, p.z),
                                0.375,
                            )
                            .map_err(|e| format!("{e:?}"))?;
                    }
                    if collision {
                        steps.push([a, b]);
                    }
                }
            }
        }
        println!("{sweep_queries} sweep capsules checked");
        let mut step_heights_dm = Vec::with_capacity(steps.len());
        for &[a, b] in &steps {
            let from = grid.cell_centre(a);
            let to = grid.cell_centre(b);
            let delta = to - from;
            let count = (delta.x.hypot(delta.z) / 0.05).ceil().max(1.0) as u32;
            let mut height = 0.0f32;
            for sample in 0..=count {
                let point = from + delta * (sample as f32 / count as f32);
                if let Some(sample_height) = collider.solid_height_above_ground(
                    point.x,
                    point.z,
                    grid.ground_height(point.x, point.z),
                    2.0,
                ) {
                    height = height.max(sample_height);
                }
            }
            step_heights_dm.push((height * 10.0).round() as u16);
        }
        grid = Grid::open_except(
            grid.origin(),
            grid.cell_m(),
            grid.cols(),
            grid.rows(),
            &blocked,
        )
        .map_err(|e| format!("{e:?}"))?
        .with_terrain(grid.terrain().ok_or("terrain")?.clone())
        .with_collision(&steps)
        .map_err(|e| format!("{e:?}"))?;
        value["grid"]["blocked"] =
            json!(blocked.iter().map(|c| [c.col, c.row]).collect::<Vec<_>>());
        value["grid"]["blocked_steps"] = json!(
            steps
                .iter()
                .map(|[a, b]| [[a.col, a.row], [b.col, b.row]])
                .collect::<Vec<_>>()
        );
        value["grid"]["step_heights_dm"] = json!(step_heights_dm);
        println!("DERIVED BLOCKED {}", value["grid"]["blocked"]);
        println!("DERIVED STEPS {}", value["grid"]["blocked_steps"]);
        println!("DERIVED STEP HEIGHTS {}", value["grid"]["step_heights_dm"]);
        ascii(&value, &grid);
        value["terrain"]["heights_cm"] = json!(
            heights
                .iter()
                .map(|h| (h * 100.0).round() as i32)
                .collect::<Vec<_>>()
        );
        for (index, layout) in value["layouts"]
            .as_array_mut()
            .ok_or("layouts")?
            .iter_mut()
            .enumerate()
        {
            let layout_grid = grid.with_layout(&scene.layouts[index]);
            let mut obstacles = BTreeSet::new();
            for obstacle in layout["obstacles"].as_array().ok_or("obstacles")? {
                for cell in obstacle["cells"].as_array().ok_or("obstacle cells")? {
                    obstacles.insert(Cell::new(
                        cell[0].as_i64().ok_or("col")? as i32,
                        cell[1].as_i64().ok_or("row")? as i32,
                    ));
                }
            }
            let free: BTreeSet<_> = layout_grid
                .walkable_cells()
                .filter(|c| !obstacles.contains(c))
                .collect();
            let largest = largest_area(&layout_grid, &free, None);
            if largest == 0 {
                return Err("no free ground".into());
            }
            let north_south = layout["id"] == "north-south";
            let axis = |cell: Cell| if north_south { cell.row } else { cell.col };
            let side = if north_south {
                grid.rows()
            } else {
                grid.cols()
            } as i32;
            let candidates: Vec<_> = free
                .iter()
                .copied()
                .filter(|c| {
                    reachable(&layout_grid, &free, *c, None).len() == largest
                        && neighbours(&layout_grid, &free, *c, None).len() >= 2
                })
                .collect();
            let connected_after_removal = largest_connectivity_after_removal(&layout_grid, &free);
            println!(
                "layout {}: {} free, {} connected",
                layout["id"],
                free.len(),
                largest
            );
            let hero_original = json_cells(&layout["spawns"]["heroes"])?;
            let enemy_original = json_cells(&layout["spawns"]["enemies"])?;
            let (hero_on_greater_bank, enemy_on_greater_bank) = if north_south {
                (true, false)
            } else {
                (false, true)
            };
            let (lesser_count, greater_count) = if hero_on_greater_bank {
                (enemy_original.len(), hero_original.len())
            } else {
                (hero_original.len(), enemy_original.len())
            };
            let (mut hero_sets, mut enemy_sets) = (0..side - 1)
                .filter(|cut| {
                    candidates.iter().filter(|c| axis(**c) <= *cut).count() >= lesser_count
                        && candidates.iter().filter(|c| axis(**c) > *cut).count() >= greater_count
                })
                .filter_map(|split| {
                    let hero_candidates: Vec<_> = candidates
                        .iter()
                        .copied()
                        .filter(|cell| {
                            if hero_on_greater_bank {
                                axis(*cell) > split
                            } else {
                                axis(*cell) <= split
                            }
                        })
                        .collect();
                    let enemy_candidates: Vec<_> = candidates
                        .iter()
                        .copied()
                        .filter(|cell| {
                            if enemy_on_greater_bank {
                                axis(*cell) > split
                            } else {
                                axis(*cell) <= split
                            }
                        })
                        .collect();
                    let hero_sets = legal_spawn_sets(
                        &layout_grid,
                        &free,
                        &hero_candidates,
                        hero_original.len(),
                        &connected_after_removal,
                    );
                    let enemy_sets = legal_spawn_sets(
                        &layout_grid,
                        &free,
                        &enemy_candidates,
                        enemy_original.len(),
                        &connected_after_removal,
                    );
                    let separated = hero_sets.iter().any(|heroes| {
                        enemy_sets.iter().any(|enemies| {
                            heroes.iter().all(|hero| {
                                enemies.iter().all(|enemy| {
                                    distance(&layout_grid, &free, *hero, *enemy)
                                        .is_some_and(|steps| steps >= 3)
                                })
                            })
                        })
                    });
                    separated.then_some((split, hero_sets, enemy_sets))
                })
                .min_by_key(|(split, _, _)| (2 * split - (side - 2)).abs())
                .map(|(_, hero_sets, enemy_sets)| (hero_sets, enemy_sets))
                .ok_or("no legal opposing spawn banks")?;
            let cost = |original: &[Cell], selected: &[Cell]| {
                assign_nearest_starts(original, selected)
                    .iter()
                    .zip(original)
                    .map(|(cell, old)| {
                        let dc = cell.col.abs_diff(old.col);
                        let dr = cell.row.abs_diff(old.row);
                        dc * dc + dr * dr
                    })
                    .sum::<u32>()
            };
            hero_sets.sort_by_key(|set| cost(&hero_original, set));
            enemy_sets.sort_by_key(|set| cost(&enemy_original, set));
            let (heroes, enemies) = hero_sets
                .iter()
                .find_map(|heroes| {
                    enemy_sets.iter().find_map(|enemies| {
                        heroes
                            .iter()
                            .all(|hero| {
                                enemies.iter().all(|enemy| {
                                    distance(&layout_grid, &free, *hero, *enemy)
                                        .is_some_and(|steps| steps >= 3)
                                })
                            })
                            .then_some((heroes, enemies))
                    })
                })
                .ok_or("no legal spawn arrangement")?;
            let heroes = assign_nearest_starts(&hero_original, heroes);
            let enemies = assign_nearest_starts(&enemy_original, enemies);
            layout["spawns"]["heroes"] = json!(
                heroes
                    .iter()
                    .map(|cell| [cell.col, cell.row])
                    .collect::<Vec<_>>()
            );
            layout["spawns"]["enemies"] = json!(
                enemies
                    .iter()
                    .map(|cell| [cell.col, cell.row])
                    .collect::<Vec<_>>()
            );
            validate_spawns(&layout_grid, &free, &heroes, &enemies)
                .map_err(|error| format!("layout {}: {error}", layout["id"]))?;
        }
        let id = value["location_id"].as_str().ok_or("scene id")?.to_owned();
        if !id.ends_with("-voxel-v1") {
            value["location_id"] = json!(format!("{id}-voxel-v1"));
        }
        // Reuse the original scene's camera occupancy at load time; never add a voxel dump.
        value
            .as_object_mut()
            .ok_or("scene object")?
            .remove("camera_occupancy");
        let authored =
            parse_scene(&value.to_string()).map_err(|e| format!("generated scene: {e:?}"))?;
        println!("AFTER ({total} splats, 0.1 m voxels, opacity 0.3)");
        ascii(&value, &authored.grid);
        println!("BLOCKED {}", value["grid"]["blocked"]);
        println!("STEPS {}", value["grid"]["blocked_steps"]);
        std::fs::write(
            &args[3],
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())? + "\n",
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}
