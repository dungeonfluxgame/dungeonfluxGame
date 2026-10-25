//! Authoring tool: writes the `camera_occupancy` data of a scan scene, the coarse map of where the
//! scan holds solid geometry (trunks, branches, foliage, walls) that the resting camera keeps its
//! lines of sight clear of.
//!
//! It reads one level of the downloaded scan (chunks decoded with the OS's `sips`), places the
//! splats with the scene's own transform and counts opaque splats per cube. Development tooling
//! only: run it natively, never in the browser.
//!
//! ```text
//! cargo run -p df-render --example splat_occupancy -- <scene id> <scan-dir (…/<id>/v1)> <level> <out.json> \
//!     [--cell 0.8] [--half 24] [--top 14] [--min-splats 6] [--min-opacity 0.3]
//! ```

#[cfg(target_arch = "wasm32")]
fn main() {}

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
    use std::{collections::BTreeSet, path::PathBuf};

    use super::scan_planes;
    use df_render::battle::{
        Focus, Limits, LodIndex, Occupancy, SplatCloud, SplatTransform, Vec3, load_embedded,
    };

    pub fn run() -> Result<(), String> {
        let mut args = std::env::args().skip(1);
        let scene_id = args.next().ok_or("scene id required")?;
        let scan: PathBuf = args.next().ok_or("scan directory required")?.into();
        let level: u32 = args
            .next()
            .ok_or("level required")?
            .parse()
            .map_err(|_| "level")?;
        let out: PathBuf = args.next().ok_or("output json required")?.into();
        let (mut cell_m, mut half_extent_m, mut top_m) = (0.8f32, 24.0f32, 14.0f32);
        let (mut min_splats, mut minimum_opacity_share) = (6u32, 0.3f32);
        while let Some(flag) = args.next() {
            let value = args.next().ok_or(format!("{flag} needs a value"))?;
            let number = |text: &str| text.parse::<f32>().map_err(|_| flag.clone());
            match flag.as_str() {
                "--cell" => cell_m = number(&value)?,
                "--half" => half_extent_m = number(&value)?,
                "--top" => top_m = number(&value)?,
                "--min-splats" => min_splats = number(&value)? as u32,
                "--min-opacity" => minimum_opacity_share = number(&value)?,
                other => return Err(format!("unknown flag {other}")),
            }
        }
        let scene = load_embedded(&scene_id)
            .ok_or("no such scene")?
            .map_err(|e| format!("{e:?}"))?;
        let layer = scene.splat.clone().ok_or("the scene has no scan")?;
        let transform = SplatTransform {
            scale: layer.scale,
            offset_y: layer.offset_y,
            rot_y_deg: layer.rot_y_deg,
            translate: layer.translate,
            orient: layer.orient,
        };
        let text =
            std::fs::read_to_string(scan.join("lod-meta.json")).map_err(|e| e.to_string())?;
        let index = LodIndex::parse(&text).map_err(|e| format!("{e:?}"))?;
        let centre = scene.grid.centre();
        let selection = index.select(
            Focus {
                centre: transform.to_scan(centre),
                radius: 1.0e6,
            },
            Limits {
                finest: level,
                coarsest: level,
                budget: u32::MAX,
            },
            None,
        );
        let files: BTreeSet<u32> = selection.picks.iter().map(|pick| pick.range.file).collect();
        eprintln!(
            "level {level}: {} splats in {} chunk file(s)",
            selection.total,
            files.len()
        );
        let files: Vec<u32> = files.into_iter().collect();
        let mut chunks = std::collections::HashMap::new();
        std::thread::scope(|threads| {
            let handles: Vec<_> = files
                .chunks(files.len().div_ceil(8).max(1))
                .map(|part| {
                    let (scan, index) = (&scan, &index);
                    threads.spawn(move || {
                        part.iter()
                            .map(|file| {
                                let folder = &index.files()[*file as usize].folder;
                                (*file, scan_planes::load_chunk(&scan.join(folder)))
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for handle in handles {
                for (file, loaded) in handle.join().unwrap() {
                    chunks.insert(file, loaded.map(|(_, splats)| splats));
                }
            }
        });
        let dims = [
            (2.0 * half_extent_m / cell_m).ceil() as u32,
            ((top_m + 1.0) / cell_m).ceil() as u32,
            (2.0 * half_extent_m / cell_m).ceil() as u32,
        ];
        let origin_m = Vec3::new(centre.x - half_extent_m, -1.0, centre.z - half_extent_m);
        let mut occupancy =
            Occupancy::empty(origin_m, cell_m, dims).map_err(|e| format!("{e:?}"))?;
        let mut counts = vec![0u32; (dims[0] * dims[1] * dims[2]) as usize];
        for pick in &selection.picks {
            let Some(Some(chunk)) = chunks.get(&pick.range.file) else {
                continue;
            };
            let (from, to) = (
                pick.range.offset as usize,
                (pick.range.offset + pick.range.count) as usize,
            );
            if to > chunk.len() {
                continue;
            }
            let (cloud, _) = SplatCloud::bake(&chunk[from..to], transform);
            for splat in cloud.splats() {
                if (splat.colour[3] as f32 / 255.0) < minimum_opacity_share {
                    continue;
                }
                // What the scene's ceiling cut removes is never drawn, so it never obstructs.
                if let Some((height, radius)) = layer.clip
                    && splat.centre.y > height
                    && (splat.centre.x - centre.x).hypot(splat.centre.z - centre.z) < radius
                {
                    continue;
                }
                if let Some((x, y, z)) = occupancy.cell_of(splat.centre) {
                    counts[((y * dims[2] + z) * dims[0] + x) as usize] += 1;
                }
            }
        }
        for y in 0..dims[1] {
            for z in 0..dims[2] {
                for x in 0..dims[0] {
                    if counts[((y * dims[2] + z) * dims[0] + x) as usize] >= min_splats {
                        occupancy.set(x, y, z);
                    }
                }
            }
        }
        eprintln!(
            "{} of {} cubes solid; hex {} bytes",
            occupancy.solid_cells(),
            counts.len(),
            occupancy.to_hex().len()
        );
        // A map from above: the lowest solid cube above 1 m in each column, in metres.
        for z in (0..dims[2]).step_by(2) {
            let row: String = (0..dims[0])
                .step_by(1)
                .map(|x| {
                    let lowest = (0..dims[1])
                        .find(|y| {
                            origin_m.y + *y as f32 * cell_m >= 1.0 && occupancy.is_set(x, *y, z)
                        })
                        .map(|y| origin_m.y + y as f32 * cell_m);
                    match lowest {
                        None => '.',
                        Some(h) if h < 2.0 => '1',
                        Some(h) if h < 4.0 => '2',
                        Some(h) if h < 8.0 => '4',
                        Some(_) => '8',
                    }
                })
                .collect();
            eprintln!("{row}");
        }
        let json = serde_json::json!({
            "origin": [origin_m.x, origin_m.y, origin_m.z],
            "cell_m": cell_m,
            "dims": dims,
            "hex": occupancy.to_hex(),
        });
        std::fs::write(
            &out,
            serde_json::to_string(&json).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        eprintln!("wrote {}", out.display());
        Ok(())
    }
}
