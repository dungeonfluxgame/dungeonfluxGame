//! A figure standing on a walkable cell is never hidden by scan splats that are beside it or behind
//! it, at any level of detail, from any of the scene's authored shots.
//!
//! This runs the real scans (the downloaded chunks, decoded with the operating system's WebP
//! decoder) through a CPU copy of the splat projection, blend and depth resolve, then asks, for
//! every walkable cell and every authored camera, how much of a figure's body the scan's depth
//! hides. It skips cleanly when the scans or `sips` are absent.
//!
//! What it holds the renderer to:
//! - at every level of detail, a figure is hidden no more than the finest drawn level hides it
//!   (plus a small bound): a coarser tier never covers a figure that the finest tier does not;
//! - without the figure rule the same scans do cover figures at the coarser levels (so the check
//!   has teeth), which is what the owner saw: a figure drawn from the waist down beside a palisade.

#[path = "support/scan_planes.rs"]
mod scan_planes;

use std::{collections::HashMap, path::Path};

use df_render::battle::{
    CameraPose, Cell, Focus, Limits, LodIndex, Mat4, ShotName, Splat, SplatCloud, SplatTransform,
    Vec3,
    figure_clear::{self, FigureBox},
    load_embedded,
};

const WIDTH: usize = 480;
const HEIGHT: usize = 270;
const ASPECT: f32 = WIDTH as f32 / HEIGHT as f32;
/// Opacity, solid-core and resolve constants of the splat passes (`splat_shaders.rs`,
/// `splat_engine.rs`).
const MIN_CONTRIBUTION: f32 = 0.45;
const RESOLVE_THRESHOLD: f32 = 0.5;
/// The figure's ground margin (`FIGURE_GROUND_MARGIN_M` in the renderer).
const GROUND_MARGIN_M: f32 = 0.65;
/// A figure is 1.35 m tall in these runs, like the preview's heroes.
const FIGURE_M: f32 = 1.35;
/// The most a coarser level may hide of a figure beyond what the finest level hides.
const EXTRA_COVER_BOUND: f32 = 0.08;

struct Projected {
    x: f32,
    y: f32,
    depth: f32,
    dir: (f32, f32),
    e1: f32,
    e2: f32,
    falloff: f32,
    alpha: f32,
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `project_splat` of the projection pass, for one splat: its screen footprint, or `None` when it
/// is culled.
fn project(splat: &Splat, view: &Mat4, view_proj: &Mat4, focal: f32) -> Option<Projected> {
    let opacity = splat.colour[3] as f32 / 255.0;
    if opacity < 1.0 / 255.0 {
        return None;
    }
    let vc = view.transform_point(splat.centre);
    if vc.z > -0.2 {
        return None;
    }
    let (ndc_x, ndc_y, _) = view_proj.project(splat.centre)?;
    let vz = vc.z.min(-0.001);
    let j1 = focal / vz;
    let (j2x, j2y) = (-j1 / vz * vc.x, -j1 / vz * vc.y);
    let m = &view.0;
    let (w0, w1, w2) = (
        Vec3::new(m[0], m[4], m[8]),
        Vec3::new(m[1], m[5], m[9]),
        Vec3::new(m[2], m[6], m[10]),
    );
    let tt0 = w0 * j1 + w2 * j2x;
    let tt1 = w1 * j1 + w2 * j2y;
    let [xx, xy, xz, yy, yz, zz] = splat.covariance;
    let quad = |a: Vec3, b: Vec3| {
        a.x * (xx * b.x + xy * b.y + xz * b.z)
            + a.y * (xy * b.x + yy * b.y + yz * b.z)
            + a.z * (xz * b.x + yz * b.y + zz * b.z)
    };
    let (a_raw, b_raw, c_raw) = (quad(tt0, tt0), quad(tt0, tt1), quad(tt1, tt1));
    let (mut a, mut b, mut c) = (a_raw + 0.3, b_raw, c_raw + 0.3);
    let det = a * c - b * b;
    if det <= 0.0 {
        return None;
    }
    let aa = ((a_raw * c_raw - b * b) / det).max(0.0).sqrt();
    if opacity * std::f32::consts::TAU * det.sqrt() < MIN_CONTRIBUTION {
        return None;
    }
    let vmin = 1024.0f32.min(WIDTH.min(HEIGHT) as f32);
    let uncapped = 2.0 * (2.0 * a.max(c)).sqrt();
    let cap = (uncapped / vmin).max(1.0);
    let inv = 1.0 / (cap * cap);
    (a, b, c) = (a * inv, b * inv, c * inv);
    let mid = 0.5 * (a + c);
    let radius = ((0.5 * (a - c)).powi(2) + b * b).sqrt();
    let (l1, l2) = (mid + radius, (mid - radius).max(0.1));
    let k = (2.0 * (255.0 * opacity).max(1.001).ln())
        .sqrt()
        .clamp(1.0, 2.828_427);
    let (e1, e2) = (k * l1.sqrt(), k * l2.sqrt());
    if e1.max(e2) < 1.0 {
        return None;
    }
    let (px, py) = (
        (ndc_x * 0.5 + 0.5) * WIDTH as f32,
        (ndc_y * 0.5 + 0.5) * HEIGHT as f32,
    );
    if px < -e1.max(e2)
        || px > WIDTH as f32 + e1.max(e2)
        || py < -e1.max(e2)
        || py > HEIGHT as f32 + e1.max(e2)
    {
        return None;
    }
    let length = (b * b + (l1 - a) * (l1 - a)).sqrt().max(1e-9);
    Some(Projected {
        x: px,
        y: py,
        depth: -vc.z,
        dir: (b / length, (l1 - a) / length),
        e1,
        e2,
        falloff: 0.5 * k * k,
        alpha: opacity * aa,
    })
}

/// One camera's projected splats, binned by screen tile, farthest first within a tile.
struct Frame {
    splats: Vec<Projected>,
    bins: HashMap<(i32, i32), Vec<u32>>,
}

const TILE: i32 = 16;

fn frame(splats: &[Splat], pose: &CameraPose) -> Frame {
    let view = pose.view();
    let view_proj = pose.projection(ASPECT, 0.1, 300.0).mul(&view);
    let focal = 0.5 * HEIGHT as f32 / (pose.fov_deg.to_radians() * 0.5).tan();
    let mut projected: Vec<Projected> = splats
        .iter()
        .filter_map(|splat| project(splat, &view, &view_proj, focal))
        .collect();
    projected.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    let mut bins: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (index, splat) in projected.iter().enumerate() {
        let reach = splat.e1.max(splat.e2);
        let (x0, x1) = (
            ((splat.x - reach).floor() as i32).div_euclid(TILE),
            ((splat.x + reach).ceil() as i32).div_euclid(TILE),
        );
        let (y0, y1) = (
            ((splat.y - reach).floor() as i32).div_euclid(TILE),
            ((splat.y + reach).ceil() as i32).div_euclid(TILE),
        );
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                bins.entry((tx, ty)).or_default().push(index as u32);
            }
        }
    }
    Frame {
        splats: projected,
        bins,
    }
}

/// The scan depth the figure at `figure` is tested against, at each pixel of `window`
/// (`x0, y0, x1, y1`, pixels, y up): blended through the splats that touch the window with the
/// figure rule applied for `exempt_rule`.
fn depth_window(
    frame: &Frame,
    window: (i32, i32, i32, i32),
    figure: Option<FigureBox>,
    bias_m: f32,
) -> Vec<f32> {
    let (x0, y0, x1, y1) = window;
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut candidates: Vec<u32> = Vec::new();
    for ty in y0.div_euclid(TILE)..=(y1 - 1).div_euclid(TILE) {
        for tx in x0.div_euclid(TILE)..=(x1 - 1).div_euclid(TILE) {
            if let Some(list) = frame.bins.get(&(tx, ty)) {
                candidates.extend_from_slice(list);
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    let mut weighted = vec![0.0f32; w * h];
    let mut covered = vec![0.0f32; w * h];
    let boxes: Vec<FigureBox> = figure.into_iter().collect();
    // `candidates` ascend in the far-to-near order of `frame.splats`.
    for index in candidates {
        let splat = &frame.splats[index as usize];
        let exempt = !boxes.is_empty() && {
            let centre = (
                splat.x / WIDTH as f32 * 2.0 - 1.0,
                splat.y / HEIGHT as f32 * 2.0 - 1.0,
            );
            let reach = splat.e1.max(splat.e2);
            let reach = (reach * 2.0 / WIDTH as f32, reach * 2.0 / HEIGHT as f32);
            figure_clear::is_exempt(centre, reach, splat.depth, &boxes)
        };
        let reach = splat.e1.max(splat.e2);
        let (px0, px1) = (
            ((splat.x - reach).floor() as i32).max(x0),
            ((splat.x + reach).ceil() as i32).min(x1),
        );
        let (py0, py1) = (
            ((splat.y - reach).floor() as i32).max(y0),
            ((splat.y + reach).ceil() as i32).min(y1),
        );
        let edge = (-splat.falloff).exp();
        for py in py0..py1 {
            for px in px0..px1 {
                let (dx, dy) = (px as f32 + 0.5 - splat.x, py as f32 + 0.5 - splat.y);
                let along = dx * splat.dir.0 + dy * splat.dir.1;
                let across = dx * splat.dir.1 - dy * splat.dir.0;
                let a = (along / splat.e1).powi(2) + (across / splat.e2).powi(2);
                if a > 1.0 {
                    continue;
                }
                let alpha = ((-splat.falloff * a).exp() - edge) / (1.0 - edge) * splat.alpha;
                if alpha < 0.0039 {
                    continue;
                }
                let solid = if exempt {
                    0.0
                } else {
                    smoothstep(0.25, 0.7, alpha)
                };
                let at = (py - y0) as usize * w + (px - x0) as usize;
                weighted[at] = splat.depth * solid + weighted[at] * (1.0 - solid);
                covered[at] = solid + covered[at] * (1.0 - solid);
            }
        }
    }
    weighted
        .iter()
        .zip(&covered)
        .map(|(sum, share)| {
            if *share < RESOLVE_THRESHOLD {
                f32::MAX
            } else {
                (sum / share + bias_m).max(0.2)
            }
        })
        .collect()
}

/// The share of a standing figure's body that the scan hides from `pose`.
fn hidden_share(
    frame: &Frame,
    pose: &CameraPose,
    plane_m: f32,
    feet: Vec3,
    bias_m: f32,
    with_rule: bool,
) -> f32 {
    let view = pose.view();
    let view_proj = pose.projection(ASPECT, 0.1, 300.0).mul(&view);
    let Some(figure) = figure_clear::figure_box(&view, &view_proj, feet, FIGURE_M) else {
        return 0.0;
    };
    let eye = pose.eye();
    let forward = (pose.target - eye).normalized();
    let right = Vec3::new(-forward.z, 0.0, forward.x).normalized();
    // The window: the figure's box in pixels.
    let to_px = |ndc: f32, size: usize| (ndc * 0.5 + 0.5) * size as f32;
    let window = (
        (to_px(figure.rect[0], WIDTH).floor() as i32).clamp(0, WIDTH as i32 - 1),
        (to_px(figure.rect[1], HEIGHT).floor() as i32).clamp(0, HEIGHT as i32 - 1),
        (to_px(figure.rect[2], WIDTH).ceil() as i32).clamp(1, WIDTH as i32),
        (to_px(figure.rect[3], HEIGHT).ceil() as i32).clamp(1, HEIGHT as i32),
    );
    if window.2 <= window.0 || window.3 <= window.1 {
        return 0.0;
    }
    let depths = depth_window(frame, window, with_rule.then_some(figure), bias_m);
    let width = (window.2 - window.0) as usize;
    let (mut hidden, mut total) = (0, 0);
    // Body samples: a grid over the middle of the figure's width, from the knees to the head.
    for column in 0..5 {
        for row in 0..9 {
            let across = (column as f32 / 4.0 - 0.5) * 0.45 * FIGURE_M;
            let up = (0.2 + 0.78 * row as f32 / 8.0) * FIGURE_M;
            let point = feet + right * across + Vec3::new(0.0, up, 0.0);
            let Some((x, y, _)) = view_proj.project(point) else {
                continue;
            };
            let (px, py) = (
                (to_px(x, WIDTH).floor() as i32),
                (to_px(y, HEIGHT).floor() as i32),
            );
            if px < window.0 || px >= window.2 || py < window.1 || py >= window.3 {
                continue;
            }
            // Where the figure is tested: its own distance, or where its line of sight crosses the
            // ground margin if that is nearer (`standing_depth`).
            let ray = point - eye;
            let own = ray.length();
            let direction = ray * (1.0 / own);
            let along = df_render::battle::grid_lines::standing_distance_m(
                eye.y,
                plane_m,
                GROUND_MARGIN_M,
                direction.y,
                own,
            );
            let standing = direction * along;
            let figure_depth = standing.dot(forward);
            let scan = depths[(py - window.1) as usize * width + (px - window.0) as usize];
            total += 1;
            hidden += usize::from(scan < figure_depth);
        }
    }
    if total == 0 {
        0.0
    } else {
        hidden as f32 / total as f32
    }
}

fn exists(root: &Path, scan: &str, folder: &str) -> bool {
    root.join(scan)
        .join("v1")
        .join(folder)
        .join("meta.json")
        .is_file()
}

#[test]
fn a_figure_is_never_hidden_by_splats_beside_or_behind_it_at_any_level_of_detail() {
    let Some(root) = scan_planes::scan_directory() else {
        eprintln!("skipped: no downloaded scans (set DF_SPLAT_SCAN_DIR)");
        return;
    };
    if std::process::Command::new("sips")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipped: sips cannot decode WebP here");
        return;
    }
    let started = std::time::Instant::now();
    let mut cases = 0;
    for (scene_id, scan) in [("scan-village", "1994c65c"), ("scan-riverbed", "9b6bc6bf")] {
        let Ok(text) = std::fs::read_to_string(root.join(scan).join("v1/lod-meta.json")) else {
            eprintln!("skipped {scene_id}: its index is not downloaded");
            continue;
        };
        let scene = load_embedded(scene_id).unwrap().unwrap();
        let layer = scene.splat.clone().unwrap();
        let index = LodIndex::parse(&text).unwrap();
        let transform = SplatTransform {
            scale: layer.scale,
            offset_y: layer.offset_y,
            rot_y_deg: layer.rot_y_deg,
            translate: layer.translate,
            orient: layer.orient,
        };
        let (finest, coarsest) = layer.levels.unwrap();
        let centre_world = scene.grid.centre();
        let derived = 0.5
            * ((scene.grid.cols() as f32 * scene.grid.cell_m()).powi(2)
                + (scene.grid.rows() as f32 * scene.grid.cell_m()).powi(2))
            .sqrt()
            + 3.0;
        let present = |file: u32| exists(&root, scan, &index.files()[file as usize].folder);
        let mut chunks: HashMap<u32, Vec<df_render::battle::RawSplat>> = HashMap::new();
        let mut covers: Vec<(u32, String, Cell, f32, f32)> = Vec::new();
        for lod in 0..=3u32 {
            let limits = Limits {
                finest: (finest + lod).min(coarsest),
                coarsest,
                budget: 2_500_000,
            };
            let radius = index.widest_full_detail_radius(
                transform.to_scan(centre_world),
                limits,
                derived / layer.scale,
                60.0 / layer.scale,
            );
            let selection = index.select(
                Focus {
                    centre: transform.to_scan(centre_world),
                    radius,
                },
                limits,
                Some(&present),
            );
            // Decode what the selection reads (in parallel: each plane is a `sips` call).
            let missing: Vec<u32> = selection
                .picks
                .iter()
                .map(|pick| pick.range.file)
                .filter(|file| !chunks.contains_key(file))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            std::thread::scope(|threads| {
                let handles: Vec<_> = missing
                    .chunks(missing.len().div_ceil(8).max(1))
                    .map(|part| {
                        let (root, index) = (&root, &index);
                        threads.spawn(move || {
                            part.iter()
                                .map(|file| {
                                    let folder = &index.files()[*file as usize].folder;
                                    let loaded = scan_planes::load_chunk(
                                        &root.join(scan).join("v1").join(folder),
                                    );
                                    (*file, loaded.map(|(_, splats)| splats))
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                for handle in handles {
                    for (file, splats) in handle.join().unwrap() {
                        chunks.insert(file, splats.unwrap_or_default());
                    }
                }
            });
            let mut baked: Vec<Splat> = Vec::new();
            for pick in &selection.picks {
                let chunk = &chunks[&pick.range.file];
                let (from, to) = (
                    pick.range.offset as usize,
                    (pick.range.offset + pick.range.count) as usize,
                );
                if to > chunk.len() {
                    continue;
                }
                let (cloud, _) = SplatCloud::bake(&chunk[from..to], transform);
                baked.extend_from_slice(cloud.splats());
            }
            // The scene's ceiling cut.
            if let Some((height, cut)) = layer.clip {
                baked.retain(|splat| {
                    let flat =
                        (splat.centre.x - centre_world.x).hypot(splat.centre.z - centre_world.z);
                    !(splat.centre.y > height && flat < cut)
                });
            }
            eprintln!(
                "{scene_id} lod {lod}: levels {}..{coarsest}, full-detail radius {:.1} m, {} splats ({:.0}s)",
                limits.finest,
                radius * layer.scale,
                baked.len(),
                started.elapsed().as_secs_f32()
            );
            for shot in [
                ShotName::Tactical,
                ShotName::TurnFocus,
                ShotName::Impact,
                ShotName::Ko,
            ] {
                let authored = scene.cameras[&shot];
                // The authored heading, pitch and lens, looking at the middle of the grid from far
                // enough that every cell is in the picture.
                let mut pose = CameraPose {
                    target: Vec3::new(centre_world.x, 0.8, centre_world.z),
                    ..authored
                };
                pose.distance = 8.0;
                while pose.distance < 40.0
                    && !scene.grid.walkable_cells().all(|cell| {
                        let feet = scene.grid.cell_centre(cell);
                        let view_proj = pose.projection(ASPECT, 0.1, 300.0).mul(&pose.view());
                        [0.0, FIGURE_M].iter().all(|height| {
                            view_proj
                                .project(feet + Vec3::new(0.0, *height, 0.0))
                                .is_some_and(|(x, y, _)| x.abs() < 0.9 && y.abs() < 0.9)
                        })
                    })
                {
                    pose.distance += 0.5;
                }
                let frame = frame(&baked, &pose);
                let frame = &frame;
                for cell in scene.grid.walkable_cells() {
                    let feet = scene.grid.cell_centre(cell);
                    let plane = scene.grid.play_plane_y();
                    let with = hidden_share(frame, &pose, plane, feet, layer.depth_bias_m, true);
                    let without =
                        hidden_share(frame, &pose, plane, feet, layer.depth_bias_m, false);
                    covers.push((lod, format!("{shot:?}"), cell, with, without));
                    cases += 1;
                }
            }
        }
        // Report, then hold the levels to the finest one.
        let mut worst_without = (0.0f32, String::new());
        let mut worst_extra = (0.0f32, String::new());
        for (lod, shot, cell, with, without) in &covers {
            let finest_with = covers
                .iter()
                .find(|(l, s, c, ..)| *l == 0 && s == shot && c == cell)
                .map_or(0.0, |entry| entry.3);
            let label = format!("{scene_id} lod {lod} {shot} cell {}:{}", cell.col, cell.row);
            if *without > worst_without.0 {
                worst_without = (*without, label.clone());
            }
            if with - finest_with > worst_extra.0 {
                worst_extra = (with - finest_with, label.clone());
            }
            assert!(
                with - finest_with <= EXTRA_COVER_BOUND,
                "{label}: {with:.2} of the figure is hidden, {finest_with:.2} at the finest level"
            );
        }
        eprintln!(
            "{scene_id}: worst cover without the rule {:.2} ({}); worst extra over the finest level with it {:.2} ({})",
            worst_without.0, worst_without.1, worst_extra.0, worst_extra.1
        );
    }
    eprintln!(
        "{cases} figure/camera cases in {:.0}s",
        started.elapsed().as_secs_f32()
    );
}
