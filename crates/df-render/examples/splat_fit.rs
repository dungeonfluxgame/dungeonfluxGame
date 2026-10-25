//! Authoring tool: fits a battle area on a downloaded Gaussian splat scan.
//!
//! Reads the finest on-disk level around the area (chunks decoded with the OS's `sips`), places
//! the splats with the scene's transform, estimates the ground height field from the splats
//! themselves, marks the cells a figure cannot stand in, prints a map for review by eye and
//! writes the `terrain` and `blocked` data a scene file takes. Development tooling only: run it
//! natively, never in the browser.
//!
//! ```text
//! cargo run -p df-render --example splat_fit -- <scan-dir> <out.json> \
//!     --orient 180,0,0 --scale 1 --offset-y 0.27 --yaw 0 --translate 0,0,0 \
//!     --grid -7,-4.9,1.4,10,7 --finest 1 [--step 0.5] [--blockers N] [--relief M] [--deviation M]
//! ```

#[cfg(target_arch = "wasm32")]
fn main() {}

// The shared support module also serves the real-data tests, which use helpers this tool does not.
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
#[path = "../tests/support/scan_planes.rs"]
mod scan_planes;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), String> {
    native::run()
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::path::PathBuf;

    use super::scan_planes;
    use df_render::battle::{
        Focus, GroundPoint, GroundSettings, Limits, LodIndex, Quat, SplatTransform, Vec3,
        WalkSettings, blocked_cells, blocker_density, estimate_height_field,
        estimate_height_field_covered, orientation_from_euler_deg,
    };

    fn numbers(text: &str, count: usize) -> Result<Vec<f32>, String> {
        let values: Vec<f32> = text
            .split(',')
            .map(|part| {
                part.trim()
                    .parse::<f32>()
                    .map_err(|e| format!("{text}: {e}"))
            })
            .collect::<Result<_, _>>()?;
        (values.len() == count)
            .then_some(values)
            .ok_or_else(|| format!("{text}: expected {count} numbers"))
    }

    pub fn run() -> Result<(), String> {
        let mut args = std::env::args().skip(1);
        let scan: PathBuf = args
            .next()
            .ok_or("scan directory (…/<id>/v1) required")?
            .into();
        let out: PathBuf = args.next().ok_or("output json required")?.into();
        let mut transform = SplatTransform::IDENTITY;
        let mut grid = (-9.0, -6.75, 1.5, 12u32, 9u32);
        let (mut finest, mut step) = (0u32, 0.5f32);
        let mut walk = WalkSettings::default();
        let mut search: Option<Vec<f32>> = None;
        while let Some(flag) = args.next() {
            let value = args.next().ok_or(format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--orient" => {
                    let v = numbers(&value, 3)?;
                    transform.orient = orientation_from_euler_deg([v[0], v[1], v[2]]);
                }
                "--scale" => transform.scale = value.parse().map_err(|_| "scale")?,
                "--offset-y" => transform.offset_y = value.parse().map_err(|_| "offset-y")?,
                "--yaw" => transform.rot_y_deg = value.parse().map_err(|_| "yaw")?,
                "--translate" => {
                    let v = numbers(&value, 3)?;
                    transform.translate = Vec3::new(v[0], v[1], v[2]);
                }
                "--grid" => {
                    let v = numbers(&value, 5)?;
                    grid = (v[0], v[1], v[2], v[3] as u32, v[4] as u32);
                }
                "--finest" => finest = value.parse().map_err(|_| "finest")?,
                "--step" => step = value.parse().map_err(|_| "step")?,
                "--search" => search = Some(numbers(&value, 5)?),
                "--blockers" => walk.blockers_per_m2 = value.parse().map_err(|_| "blockers")?,
                "--relief" => walk.maximum_relief_m = value.parse().map_err(|_| "relief")?,
                "--deviation" => {
                    walk.maximum_deviation_m = value.parse().map_err(|_| "deviation")?;
                }
                other => return Err(format!("unknown flag {other}")),
            }
        }
        let _ = Quat::IDENTITY;
        let (ox, oz, cell, cols, rows) = grid;
        let width = cols as f32 * cell;
        let depth = rows as f32 * cell;
        let centre = Vec3::new(ox + width * 0.5, 0.0, oz + depth * 0.5);

        let index_text =
            std::fs::read_to_string(scan.join("lod-meta.json")).map_err(|e| e.to_string())?;
        let index = LodIndex::parse(&index_text).map_err(|e| format!("{e:?}"))?;
        let focus = Focus {
            centre: transform.to_scan(centre),
            radius: (0.5 * (width * width + depth * depth).sqrt() + 3.0) / transform.scale,
        };
        let limits = Limits {
            finest,
            coarsest: finest,
            budget: u32::MAX,
        };
        let selection = index.select(focus, limits, None);
        let plan = selection.plan(1 << 20);
        eprintln!(
            "level {finest}: {} splats in {} chunk(s)",
            plan.total,
            plan.jobs.len()
        );

        if let Some(window) = &search {
            return search_arenas(&scan, &index, transform, grid, finest, window, &walk);
        }
        let margin = 2.0;
        let (min_x, max_x) = (ox - margin, ox + width + margin);
        let (min_z, max_z) = (oz - margin, oz + depth + margin);
        let mut points: Vec<GroundPoint> = Vec::new();
        for job in &plan.jobs {
            let folder = scan.join(&index.files()[job.file as usize].folder);
            let (_, splats) = scan_planes::load_chunk(&folder)
                .ok_or_else(|| format!("cannot read chunk {}", folder.display()))?;
            for range in
                &plan.ranges[job.range_base as usize..(job.range_base + job.range_count) as usize]
            {
                let from = range[0] as usize;
                for splat in splats.iter().skip(from).take(range[1] as usize) {
                    let world = transform.to_world(splat.position);
                    if (min_x..=max_x).contains(&world.x) && (min_z..=max_z).contains(&world.z) {
                        points.push(GroundPoint {
                            position: world,
                            opacity: splat.colour[3] as f32 / 255.0,
                        });
                    }
                }
            }
        }
        eprintln!("{} splats inside the area", points.len());

        let terrain_origin = [ox - 1.0, oz - 1.0];
        let t_cols = ((width + 2.0) / step).ceil() as u32 + 1;
        let t_rows = ((depth + 2.0) / step).ceil() as u32 + 1;
        let field = estimate_height_field(
            &points,
            terrain_origin,
            step,
            t_cols,
            t_rows,
            &GroundSettings::default(),
        )
        .map_err(|e| format!("{e:?}"))?;
        let blocked = blocked_cells(&field, &points, [ox, oz], cell, cols, rows, &walk);

        let mut centres: Vec<f32> = (0..rows)
            .flat_map(|row| (0..cols).map(move |col| (col, row)))
            .map(|(col, row)| {
                field.height_at(
                    ox + (col as f32 + 0.5) * cell,
                    oz + (row as f32 + 0.5) * cell,
                )
            })
            .collect();
        centres.sort_by(f32::total_cmp);
        let median_m = centres[centres.len() / 2];
        eprintln!(
            "ground height at the cell centres: min {:.2} median {median_m:.2} max {:.2} (offset_y {:.2} -> suggest {:.2})",
            centres[0],
            centres[centres.len() - 1],
            transform.offset_y,
            transform.offset_y - median_m
        );
        let density = blocker_density(&field, &points, [ox, oz], cell, cols, rows, &walk);
        eprintln!(
            "blockers per m2 (thousands; threshold {:.1}):",
            walk.blockers_per_m2 / 1000.0
        );
        for row in 0..rows as usize {
            let line: Vec<String> = (0..cols as usize)
                .map(|col| format!("{:5.2}", density[row * cols as usize + col] / 1000.0))
                .collect();
            eprintln!("{}", line.join(" "));
        }
        eprintln!("blocked: {} of {} cells", blocked.len(), cols * rows);
        for row in 0..rows as i32 {
            let mut line = String::new();
            for col in 0..cols as i32 {
                let centre = field.height_at(
                    ox + (col as f32 + 0.5) * cell,
                    oz + (row as f32 + 0.5) * cell,
                );
                if blocked.contains(&[col, row]) {
                    line.push_str("  ## ");
                } else {
                    line.push_str(&format!("{:+4.0} ", centre * 100.0));
                }
            }
            eprintln!("{line}");
        }
        let heights_cm: Vec<i32> = field
            .heights()
            .iter()
            .map(|h| (h * 100.0).round() as i32)
            .collect();
        let json = serde_json::json!({
            "terrain": {
                "origin": terrain_origin,
                "step_m": step,
                "cols": t_cols,
                "rows": t_rows,
                "heights_cm": heights_cm,
            },
            "blocked": blocked,
            "ground_mean_m": median_m,
        });
        std::fs::write(
            &out,
            serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        eprintln!("wrote {}", out.display());
        Ok(())
    }

    /// Slides the grid over the scan and reports the open, flat windows: fewest blocked cells, the
    /// largest connected walkable area and the smallest height range. Window positions are in the
    /// transform's world frame (before `translate`): to put a window under the grid, translate
    /// by minus its centre.
    fn search_arenas(
        scan: &std::path::Path,
        index: &LodIndex,
        transform: SplatTransform,
        grid: (f32, f32, f32, u32, u32),
        finest: u32,
        window: &[f32],
        walk: &WalkSettings,
    ) -> Result<(), String> {
        let (_, _, cell, cols, rows) = grid;
        let (x0, x1, z0, z1, step) = (window[0], window[1], window[2], window[3], window[4]);
        let (width, depth) = (cols as f32 * cell, rows as f32 * cell);
        let centre = Vec3::new(0.5 * (x0 + x1), 0.0, 0.5 * (z0 + z1));
        let radius = 0.5 * ((x1 - x0 + width).hypot(z1 - z0 + depth)) + 3.0;
        let focus = Focus {
            centre: transform.to_scan(centre),
            radius: radius / transform.scale,
        };
        let limits = Limits {
            finest,
            coarsest: finest,
            budget: u32::MAX,
        };
        let plan = index.select(focus, limits, None).plan(1 << 20);
        let mut points: Vec<GroundPoint> = Vec::new();
        for job in &plan.jobs {
            let folder = scan.join(&index.files()[job.file as usize].folder);
            let (_, splats) = scan_planes::load_chunk(&folder)
                .ok_or_else(|| format!("cannot read chunk {}", folder.display()))?;
            let ranges =
                &plan.ranges[job.range_base as usize..(job.range_base + job.range_count) as usize];
            for range in ranges {
                for splat in splats
                    .iter()
                    .skip(range[0] as usize)
                    .take(range[1] as usize)
                {
                    let world = transform.to_world(splat.position);
                    let inside = world.x > x0 - 2.0
                        && world.x < x1 + width + 2.0
                        && world.z > z0 - 2.0
                        && world.z < z1 + depth + 2.0;
                    if inside {
                        points.push(GroundPoint {
                            position: world,
                            opacity: splat.colour[3] as f32 / 255.0,
                        });
                    }
                }
            }
        }
        eprintln!("{} splats in the search area", points.len());
        let mut buckets: std::collections::HashMap<(i32, i32), Vec<usize>> = Default::default();
        for (position, point) in points.iter().enumerate() {
            buckets
                .entry((
                    point.position.x.floor() as i32,
                    point.position.z.floor() as i32,
                ))
                .or_default()
                .push(position);
        }
        let mut results: Vec<Window> = Vec::new();
        let mut wz = z0;
        while wz <= z1 {
            let mut wx = x0;
            while wx <= x1 {
                let (min_x, max_x, min_z, max_z) =
                    (wx - 1.0, wx + width + 1.0, wz - 1.0, wz + depth + 1.0);
                let mut local: Vec<GroundPoint> = Vec::new();
                for bx in min_x.floor() as i32..=max_x.floor() as i32 {
                    for bz in min_z.floor() as i32..=max_z.floor() as i32 {
                        if let Some(list) = buckets.get(&(bx, bz)) {
                            local.extend(list.iter().map(|i| points[*i]));
                        }
                    }
                }
                let t_cols = ((width + 2.0) / 0.5).ceil() as u32 + 1;
                let t_rows = ((depth + 2.0) / 0.5).ceil() as u32 + 1;
                let settings = GroundSettings::default();
                let estimated = estimate_height_field_covered(
                    &local,
                    [wx - 1.0, wz - 1.0],
                    0.5,
                    t_cols,
                    t_rows,
                    &settings,
                );
                if let Ok((field, covered)) = estimated
                    && covered > 0.6
                {
                    let blocked = blocked_cells(&field, &local, [wx, wz], cell, cols, rows, walk);
                    let open = largest_open_region(&blocked, cols as i32, rows as i32);
                    let centres: Vec<f32> = (0..rows)
                        .flat_map(|r| (0..cols).map(move |c| (c, r)))
                        .map(|(c, r)| {
                            field.height_at(
                                wx + (c as f32 + 0.5) * cell,
                                wz + (r as f32 + 0.5) * cell,
                            )
                        })
                        .collect();
                    let (low, high) = centres
                        .iter()
                        .fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
                    let mean = centres.iter().sum::<f32>() / centres.len() as f32;
                    // Foliage and branches between 1.8 and 28 m up, over the grid and the 14 m on the
                    // camera side of it (the default cameras sit toward +z): what the view crosses.
                    let area = width * (depth + 14.0);
                    let overhead = local
                        .iter()
                        .filter(|p| {
                            p.opacity > 0.5
                                && p.position.x >= wx
                                && p.position.x <= wx + width
                                && p.position.z >= wz
                                && p.position.z <= wz + depth + 14.0
                        })
                        .filter(|p| {
                            let above = p.position.y - field.height_at(p.position.x, p.position.z);
                            (1.8..28.0).contains(&above)
                        })
                        .count() as f32
                        / area;
                    let score = blocked.len() as f32 - open as f32 * 0.5
                        + (high - low) * 4.0
                        + overhead * 0.02;
                    results.push(Window {
                        score,
                        wx,
                        wz,
                        blocked: blocked.len(),
                        open,
                        mean,
                        overhead,
                        range: high - low,
                    });
                }
                wx += step;
            }
            wz += step;
        }
        results.sort_by(|a, b| a.score.total_cmp(&b.score));
        eprintln!(
            "best windows (score, window origin, centre, blocked cells, largest open region, ground):"
        );
        let mut shown: Vec<(f32, f32)> = Vec::new();
        for Window {
            score,
            wx,
            wz,
            blocked,
            open,
            mean,
            overhead,
            range,
        } in results
        {
            if shown
                .iter()
                .any(|(x, z)| (x - wx).abs() < 6.0 && (z - wz).abs() < 6.0)
            {
                continue;
            }
            shown.push((wx, wz));
            eprintln!(
                "{score:7.1} origin ({wx:7.2},{wz:7.2}) centre ({:7.2},{:7.2}) blocked {blocked:3} open {open:3} ground {mean:5.2} range {range:4.2} overhead {overhead:6.0}/m2",
                wx + width * 0.5,
                wz + depth * 0.5
            );
            if shown.len() >= 8 {
                break;
            }
        }
        Ok(())
    }

    /// One candidate arena position and how it scored.
    struct Window {
        score: f32,
        wx: f32,
        wz: f32,
        blocked: usize,
        open: usize,
        mean: f32,
        overhead: f32,
        range: f32,
    }

    fn largest_open_region(blocked: &[[i32; 2]], cols: i32, rows: i32) -> usize {
        let mut seen = vec![false; (cols * rows) as usize];
        let is_blocked = |c: i32, r: i32| blocked.contains(&[c, r]);
        let mut best = 0;
        for row in 0..rows {
            for col in 0..cols {
                if seen[(row * cols + col) as usize] || is_blocked(col, row) {
                    continue;
                }
                let mut stack = vec![(col, row)];
                seen[(row * cols + col) as usize] = true;
                let mut size = 0;
                while let Some((c, r)) = stack.pop() {
                    size += 1;
                    for (dc, dr) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let (nc, nr) = (c + dc, r + dr);
                        let fresh = (0..cols).contains(&nc)
                            && (0..rows).contains(&nr)
                            && !seen[(nr * cols + nc) as usize]
                            && !is_blocked(nc, nr);
                        if fresh {
                            seen[(nr * cols + nc) as usize] = true;
                            stack.push((nc, nr));
                        }
                    }
                }
                best = best.max(size);
            }
        }
        best
    }
}
