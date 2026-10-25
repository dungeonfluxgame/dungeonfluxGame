//! Checks the SOG v2 decode against statistics of a REAL chunk of a downloaded scan.
//! Skips cleanly when the scans are not on disk (CI) or `sips` cannot decode WebP.
//!
//! The expected numbers were computed by an independent decoder (numpy, written from the
//! PlayCanvas engine's SOG shader chunks) over the same planes.

#[path = "support/scan_planes.rs"]
mod scan_planes;

use df_render::battle::{LodIndex, Vec3};

fn chunk_or_skip(
    scene: &str,
    chunk: &str,
) -> Option<(df_render::battle::SogMeta, Vec<df_render::battle::RawSplat>)> {
    let Some(root) = scan_planes::scan_directory() else {
        eprintln!("skipped: no downloaded scans (set DF_SPLAT_SCAN_DIR)");
        return None;
    };
    let folder = root.join(scene).join("v1").join(chunk);
    if !folder.join("meta.json").is_file() {
        eprintln!("skipped: {} not downloaded", folder.display());
        return None;
    }
    let loaded = scan_planes::load_chunk(&folder);
    if loaded.is_none() {
        eprintln!("skipped: could not decode planes of {}", folder.display());
    }
    loaded
}

#[test]
fn a_real_lossy_chunk_decodes_to_plausible_positions_scales_colours_and_rotations() {
    let Some((meta, splats)) = chunk_or_skip("9b6bc6bf", "3_0") else {
        return;
    };
    assert_eq!(splats.len(), meta.count);
    assert!(splats.len() > 100_000);

    let splat_count = splats.len() as f32;
    // Colour: this autumn scene is bright and warm; the independent decoder's means are
    // (0.733, 0.724, 0.660) over the first chunk.
    let mean = |channel: usize| {
        splats
            .iter()
            .map(|s| s.colour[channel] as f32 / 255.0)
            .sum::<f32>()
            / splat_count
    };
    for (channel, expected) in [(0, 0.733), (1, 0.724), (2, 0.660)] {
        assert!(
            (mean(channel) - expected).abs() < 0.02,
            "channel {channel} mean {} vs {expected}",
            mean(channel)
        );
    }
    // Opacity: a spread of values, median near 0.27 (independent decoder: 0.2745).
    let mut opacity: Vec<f32> = splats.iter().map(|s| s.colour[3] as f32 / 255.0).collect();
    opacity.sort_by(f32::total_cmp);
    let median = opacity[opacity.len() / 2];
    assert!((median - 0.2745).abs() < 0.02, "median opacity {median}");
    assert!(opacity[opacity.len() - 1] > 0.95 && opacity[0] < 0.1);

    // Rotations are unit quaternions.
    for splat in splats.iter().step_by(97) {
        let q = splat.rotation;
        let length = (q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w).sqrt();
        assert!((length - 1.0).abs() < 1e-3, "{length}");
    }
    // Scales: positive, finite; the median axis lengths in scene units are around 0.15 / 0.11 / 0.05
    // (independent decoder), and the middle of the distribution is not noise-sized or huge.
    let mut major: Vec<f32> = splats
        .iter()
        .map(|s| s.scale.x.max(s.scale.y).max(s.scale.z))
        .collect();
    assert!(major.iter().all(|v| v.is_finite() && *v >= 0.0));
    major.sort_by(f32::total_cmp);
    let middle = major[major.len() / 2];
    assert!((0.05..0.6).contains(&middle), "median major scale {middle}");

    // Positions are finite, and the bulk of them sit within the scene's tree bounds: the
    // independent decoder's 1st-99th percentile spans x -52..57, y -33..15, z -54..37.
    assert!(splats.iter().all(|s| s.position.is_finite()));
    let mut xs: Vec<f32> = splats.iter().map(|s| s.position.x).collect();
    xs.sort_by(f32::total_cmp);
    let (low, high) = (xs[xs.len() / 100], xs[xs.len() * 99 / 100]);
    assert!(
        (low + 52.3).abs() < 2.0 && (high - 56.7).abs() < 2.0,
        "x percentiles {low} {high}"
    );
}

#[test]
fn a_real_lod_index_parses_and_selects_within_budget_around_a_point() {
    let Some(root) = scan_planes::scan_directory() else {
        eprintln!("skipped: no downloaded scans");
        return;
    };
    let path = root.join("9b6bc6bf/v1/lod-meta.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipped: {} absent", path.display());
        return;
    };
    let index = LodIndex::parse(&text).unwrap();
    assert_eq!(index.level_count(), 4);
    assert!(index.leaf_count() > 100);
    let limits = df_render::battle::Limits {
        finest: 1,
        coarsest: 3,
        budget: 1_500_000,
    };
    let focus = df_render::battle::Focus {
        centre: Vec3::new(0.0, -17.0, 0.0),
        radius: 8.0,
    };
    let selection = index.select(focus, limits, None);
    assert!(selection.total <= 1_500_000, "{}", selection.total);
    assert!(selection.total > 100_000);
    let plan = selection.plan(df_render::battle::MAX_RANGES);
    assert_eq!(plan.total, selection.total);
    assert!(!plan.truncated);
    // Everything planned lives in a chunk file the index names.
    assert!(
        plan.jobs
            .iter()
            .all(|job| (job.file as usize) < index.files().len())
    );
}
