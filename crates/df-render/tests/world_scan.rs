//! Checks the PLY loader and the world grid against a REAL World Labs Marble world saved by the
//! research pass (an old cloth merchant's shop): the 100k-splat PLY and the collider GLB.
//! Skips cleanly when the files are not on disk (CI). Nothing here calls World Labs.
//!
//! The scale factor and ground offset are the world's `semantics_metadata` from
//! `m11.operation.json`. The expected counts were measured with this code on the saved files.
//! Over the splats' own extent at half-metre squares it reproduces the research harness's
//! 82 open, 87 blocked and 21 without floor out of 190 (see the test of that name).

use std::path::{Path, PathBuf};

use df_render::battle::{
    ColliderMesh, PlyLoader, RawSplat, SplatLoader, SplatTransform, Vec3, WorldBounds, WorldCell,
    WorldGrid, WorldGridSettings, marble_world_transform, world_grid,
};

const METRIC_SCALE_FACTOR: f32 = 1.501_432_2;
const GROUND_PLANE_OFFSET: f32 = 1.208_879_7;

fn saved_world() -> Option<PathBuf> {
    let directory = std::env::var_os("DF_WORLDLABS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/tmp/worldlabs/out/m11")
        });
    if directory.join("m11_100k.ply").is_file() && directory.join("m11_collider.glb").is_file() {
        Some(directory)
    } else {
        eprintln!(
            "skipped: no saved Marble world in {} (set DF_WORLDLABS_DIR)",
            directory.display()
        );
        None
    }
}

fn load_splats(directory: &Path) -> Vec<RawSplat> {
    let bytes = std::fs::read(directory.join("m11_100k.ply")).unwrap();
    PlyLoader { bytes: &bytes }.load().unwrap()
}

fn load_mesh(directory: &Path) -> ColliderMesh {
    let bytes = std::fs::read(directory.join("m11_collider.glb")).unwrap();
    ColliderMesh::from_glb(&bytes).unwrap()
}

/// The grid as text, far rows first: `.` open, `#` blocked, `?` no floor.
fn picture(grid: &WorldGrid) -> String {
    let mut text = String::new();
    for row in (0..grid.rows()).rev() {
        for col in 0..grid.cols() {
            text.push(match grid.cell(col, row) {
                Some(WorldCell::Open) => '.',
                Some(WorldCell::Blocked) => '#',
                _ => '?',
            });
        }
        text.push('\n');
    }
    text
}

#[test]
fn the_saved_ply_loads_with_the_expected_count_and_stands_on_the_floor() {
    let Some(directory) = saved_world() else {
        return;
    };
    let splats = load_splats(&directory);
    assert_eq!(splats.len(), 98_304);
    for splat in &splats {
        let q = splat.rotation;
        let length = (q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w).sqrt();
        assert!((length - 1.0).abs() < 1e-3);
        assert!(splat.scale.x > 0.0 && splat.scale.y > 0.0 && splat.scale.z > 0.0);
    }
    // A fair share of the splats are solid, and a fair share are not.
    let solid = splats.iter().filter(|s| s.colour[3] > 128).count();
    assert!(
        (splats.len() / 10..splats.len()).contains(&solid),
        "{solid}"
    );

    // Stood upright with the world's own numbers, the opaque splats start at the floor (y = 0)
    // and the shop's ceiling beams are a few metres above it.
    let transform = marble_world_transform(METRIC_SCALE_FACTOR, GROUND_PLANE_OFFSET).unwrap();
    let mut heights: Vec<f32> = splats
        .iter()
        .filter(|s| s.colour[3] > 128)
        .map(|s| transform.to_world(s.position).y)
        .collect();
    heights.sort_by(f32::total_cmp);
    let low = heights[heights.len() / 100];
    let high = heights[heights.len() - 1 - heights.len() / 100];
    assert!((-0.3..0.3).contains(&low), "p1 height {low}");
    assert!((2.0..6.0).contains(&high), "p99 height {high}");
}

/// The research harness's footprint: the 2nd to 98th percentile of the opaque splats' world x and
/// z, which is the shop without the street.
fn splat_extent(splats: &[RawSplat], transform: &SplatTransform) -> WorldBounds {
    let centres: Vec<Vec3> = splats
        .iter()
        .filter(|s| s.colour[3] > 128)
        .map(|s| transform.to_world(s.position))
        .collect();
    let percentile = |axis: fn(&Vec3) -> f32, q: f32| {
        let mut values: Vec<f32> = centres.iter().map(axis).collect();
        values.sort_by(f32::total_cmp);
        values[((values.len() - 1) as f32 * q) as usize]
    };
    WorldBounds {
        min: [percentile(|v| v.x, 0.02), percentile(|v| v.z, 0.02)],
        max: [percentile(|v| v.x, 0.98), percentile(|v| v.z, 0.98)],
    }
}

/// Which way up the export is. The docs say OpenCV (y down), release notes say OpenGL (y up)
/// since December 2025. The world's own numbers settle it: its ground plane offset puts the
/// floor at raw y = offset / scale = +0.805, and the collider's biggest horizontal surface is
/// there, at the high-y end of the mesh. In an OpenGL export the floor would be at -0.805.
#[test]
fn the_saved_export_is_in_the_opencv_frame_and_the_transform_stands_it_upright() {
    let Some(directory) = saved_world() else {
        return;
    };
    let mesh = load_mesh(&directory);
    let splats = load_splats(&directory);
    let transform = marble_world_transform(METRIC_SCALE_FACTOR, GROUND_PLANE_OFFSET).unwrap();
    let floor_raw_y = GROUND_PLANE_OFFSET / METRIC_SCALE_FACTOR;

    // Area of horizontal surface (normal within about 18 degrees of vertical) by raw height, in
    // 5 cm bins, and the total area by world height.
    let mut horizontal = std::collections::BTreeMap::<i32, f64>::new();
    let (mut total, mut below_floor, mut weighted_height) = (0.0f64, 0.0f64, 0.0f64);
    for at in 0..mesh.triangles().len() {
        let [a, b, c] = mesh.corners(at).unwrap();
        let normal = (b - a).cross(c - a);
        let area = f64::from(normal.length()) * 0.5;
        if area <= 1e-9 {
            continue;
        }
        if (normal.y / normal.length()).abs() >= 0.95 {
            let height = (a.y + b.y + c.y) / 3.0;
            *horizontal
                .entry((height / 0.05).floor() as i32)
                .or_default() += area;
        }
        let world_height = transform.to_world((a + b + c) * (1.0 / 3.0)).y;
        total += area;
        weighted_height += area * f64::from(world_height);
        if world_height < -0.3 {
            below_floor += area;
        }
    }
    let area_near = |raw_y: f32| -> f64 {
        horizontal
            .iter()
            .filter(|(bin, _)| ((**bin as f32 + 0.5) * 0.05 - raw_y).abs() <= 0.1)
            .map(|(_, area)| *area)
            .sum()
    };
    // The biggest horizontal surface on each side of the camera: the floor is the one below it
    // (raw +y, y being down) and the ceiling the one above.
    let biggest = |range: std::ops::RangeInclusive<f32>| -> f32 {
        let (bin, _) = horizontal
            .iter()
            .filter(|(bin, _)| range.contains(&((**bin as f32 + 0.5) * 0.05)))
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        (*bin as f32 + 0.5) * 0.05
    };
    let peak = biggest(0.0..=10.0);
    assert!(
        (peak - floor_raw_y).abs() <= 0.06,
        "the biggest surface below the camera is at raw y {peak}, the floor should be at {floor_raw_y}"
    );
    let ceiling_world = transform
        .to_world(Vec3::new(0.0, biggest(-10.0..=0.0), 0.0))
        .y;
    assert!(
        (2.4..3.6).contains(&ceiling_world),
        "ceiling at world y {ceiling_world}"
    );
    let floor_area = area_near(floor_raw_y);
    assert!(floor_area > 10.0, "floor area {floor_area} m2");
    assert!(
        area_near(-floor_raw_y) < 0.25 * floor_area,
        "OpenGL would put the floor at {}: {} m2 there against {floor_area} m2",
        -floor_raw_y,
        area_near(-floor_raw_y)
    );

    // After the transform the floor is flat at y = 0 and the room is above it: almost no
    // surface is below the floor and the mean surface height is in the room.
    let floor_world = transform.to_world(Vec3::new(0.0, peak, 0.0)).y;
    assert!(floor_world.abs() <= 0.1, "floor at world y {floor_world}");
    assert!(
        below_floor < 0.05 * total,
        "{below_floor} of {total} m2 below the floor"
    );
    assert!(
        (0.8..3.5).contains(&(weighted_height / total)),
        "{}",
        weighted_height / total
    );

    // The splats agree: their lowest solid layer is the floor, with the ceiling well above.
    let mut heights: Vec<f32> = splats
        .iter()
        .filter(|s| s.colour[3] > 128)
        .map(|s| transform.to_world(s.position).y)
        .collect();
    heights.sort_by(f32::total_cmp);
    assert!(heights[heights.len() / 100].abs() < 0.3);
    assert!(heights[heights.len() / 2] > 0.3);
}

#[test]
fn the_saved_collider_reads_with_the_expected_triangle_count() {
    let Some(directory) = saved_world() else {
        return;
    };
    let mesh = load_mesh(&directory);
    assert_eq!(mesh.triangles().len(), 95_288);
    assert_eq!(mesh.vertices().len(), 48_058);
}

#[test]
fn the_saved_world_reads_as_a_cramped_shop_with_a_walkable_floor_in_the_measured_range() {
    let Some(directory) = saved_world() else {
        return;
    };
    let mesh = load_mesh(&directory);
    let transform = marble_world_transform(METRIC_SCALE_FACTOR, GROUND_PLANE_OFFSET).unwrap();

    // Half-metre squares, the research harness's resolution.
    let fine = WorldGridSettings {
        cell_m: 0.5,
        ..WorldGridSettings::default()
    };
    let grid = world_grid(&mesh, &transform, &fine, None).unwrap();
    let counts = grid.counts();
    let note = picture(&grid);
    assert_eq!((grid.cols(), grid.rows()), (35, 37), "{note}");
    assert!((95..=125).contains(&counts.open), "{counts:?}\n{note}");
    assert!((215..=275).contains(&counts.blocked), "{counts:?}\n{note}");
    assert!((900..=980).contains(&counts.no_floor), "{counts:?}\n{note}");
    let area = grid.largest_open_area().unwrap();
    // The shop floor: about 18 square metres in a box of roughly 9 x 14 half-metre squares.
    assert!((60..=85).contains(&area.squares), "{area:?}\n{note}");
    assert!((15.0..22.0).contains(&area.area_m2), "{area:?}");
    assert!(
        area.extent_squares[0] >= 7 && area.extent_squares[1] >= 11,
        "{area:?}"
    );
    let [low, high] = grid.mesh_bounds();
    assert!((-0.3..0.1).contains(&low.y) && (4.0..8.0).contains(&high.y));

    // The default five-foot squares: the shop is too tight for more than a handful of them.
    let default = WorldGridSettings::default();
    let five_foot = world_grid(&mesh, &transform, &default, None).unwrap();
    let note = picture(&five_foot);
    assert_eq!((five_foot.cols(), five_foot.rows()), (12, 13), "{note}");
    let counts = five_foot.counts();
    assert!((5..=12).contains(&counts.open), "{counts:?}\n{note}");
    assert!((45..=60).contains(&counts.blocked), "{counts:?}\n{note}");
    let area = five_foot.largest_open_area().unwrap();
    assert!((3..=8).contains(&area.squares), "{area:?}\n{note}");
    assert_eq!(five_foot.walkable_indices().len() as u32, counts.open);
}

#[test]
fn the_splat_probe_only_ever_adds_blocked_squares_on_the_saved_world() {
    let Some(directory) = saved_world() else {
        return;
    };
    let mesh = load_mesh(&directory);
    let splats = load_splats(&directory);
    let transform = marble_world_transform(METRIC_SCALE_FACTOR, GROUND_PLANE_OFFSET).unwrap();
    let settings = WorldGridSettings::default();

    let mesh_only = world_grid(&mesh, &transform, &settings, None).unwrap();
    let checked = world_grid(&mesh, &transform, &settings, Some(&splats)).unwrap();
    let note = picture(&checked);

    // The same squares, and every square the mesh blocked is still blocked.
    assert_eq!(
        (checked.cols(), checked.rows()),
        (mesh_only.cols(), mesh_only.rows())
    );
    for row in 0..checked.rows() {
        for col in 0..checked.cols() {
            let before = mesh_only.cell(col, row).unwrap();
            let after = checked.cell(col, row).unwrap();
            if before == WorldCell::Blocked {
                assert_eq!(after, WorldCell::Blocked, "({col}, {row})");
            }
            if after != before {
                assert_eq!(after, WorldCell::Blocked, "({col}, {row})");
            }
        }
    }
    // The two sources do disagree here (the probe is the noisier one), and they also agree on
    // most of the walls.
    let check = checked.cross_check().unwrap();
    assert!(check.blocked_by_both >= 30, "{check:?}\n{note}");
    assert!(
        (10..=40).contains(&check.blocked_by_probe_only),
        "{check:?}\n{note}"
    );
    assert!(check.blocked_by_mesh_only <= 15, "{check:?}\n{note}");
    // Blocked wins: every square the probe alone blocked is one more blocked square.
    assert_eq!(
        checked.counts().blocked,
        mesh_only.counts().blocked + check.blocked_by_probe_only,
        "{note}"
    );
}

#[test]
fn over_the_splat_extent_the_grid_reproduces_the_research_harness_counts() {
    let Some(directory) = saved_world() else {
        return;
    };
    let mesh = load_mesh(&directory);
    let splats = load_splats(&directory);
    let transform = marble_world_transform(METRIC_SCALE_FACTOR, GROUND_PLANE_OFFSET).unwrap();
    // The harness blocked a square at 0.15 m2 of surface in the 0.3 to 1.7 m band; as a share
    // of a wall across a half-metre square that is 0.15 / (0.5 * 1.4).
    let settings = WorldGridSettings {
        cell_m: 0.5,
        blocking_wall_share: 0.15 / (0.5 * 1.4),
        footprint: Some(splat_extent(&splats, &transform)),
        ..WorldGridSettings::default()
    };
    let grid = world_grid(&mesh, &transform, &settings, None).unwrap();
    let counts = grid.counts();
    // The same footprint gives the same 190 squares. This grid samples each triangle at the
    // centres of a lattice of sub-triangles where the harness sampled lattice corners, so a
    // square on the edge of a threshold can go either way: it measured 81, 85 and 24, within
    // three squares of the harness's 82, 87 and 21 in each class.
    assert_eq!((grid.cols(), grid.rows()), (10, 19));
    let near = |found: u32, harness: u32| found.abs_diff(harness) <= 3;
    assert!(
        near(counts.open, 82) && near(counts.blocked, 87) && near(counts.no_floor, 21),
        "{counts:?}
{}",
        picture(&grid)
    );
}
