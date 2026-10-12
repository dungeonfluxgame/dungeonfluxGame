//! Development-only access to downloaded scans: decodes the WebP planes of a SOG chunk with the
//! operating system's `sips` (macOS), reads the chunk into splats, and finds the data directory.
//! Nothing here ships; tests that use it skip cleanly when the data or `sips` is absent.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use df_render::battle::{RawSplat, SogLoader, SogMeta, SogPlanes, SplatLoader};

/// Where scans were downloaded: `DF_SPLAT_SCAN_DIR`, else `artifacts/tmp/splat-dl` above the crate.
pub fn scan_directory() -> Option<PathBuf> {
    let from_env = std::env::var_os("DF_SPLAT_SCAN_DIR").map(PathBuf::from);
    let beside = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/tmp/splat-dl");
    from_env
        .into_iter()
        .chain([beside])
        .find(|directory| directory.is_dir())
}

fn scratch_file(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("df-scan-plane-{}-{label}.bmp", std::process::id()))
}

/// Decodes a WebP file to straight (non-premultiplied) RGBA bytes with its size.
pub fn decode_webp(path: &Path) -> Option<(Vec<u8>, usize, usize)> {
    let out = scratch_file(&path.to_string_lossy().replace(['/', '.'], "_"));
    let status = Command::new("sips")
        .args(["-s", "format", "bmp"])
        .arg(path)
        .arg("--out")
        .arg(&out)
        .output()
        .ok()?;
    if !status.status.success() {
        return None;
    }
    let bytes = std::fs::read(&out).ok()?;
    let _ = std::fs::remove_file(&out);
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let offset = word(10)? as usize;
    let width = word(18)? as i32;
    let height = word(22)? as i32;
    let (width, rows, top_down) = (width as usize, height.unsigned_abs() as usize, height < 0);
    let pixels = bytes.get(offset..offset + width * rows * 4)?;
    let mut rgba = Vec::with_capacity(pixels.len());
    for row in 0..rows {
        let source = if top_down { row } else { rows - 1 - row };
        for pixel in pixels[source * width * 4..(source + 1) * width * 4].chunks_exact(4) {
            // BMP stores BGRA.
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    Some((rgba, width, rows))
}

/// Every splat of one chunk folder (`.../v1/<level>_<chunk>`), or `None` if it cannot be read.
pub fn load_chunk(folder: &Path) -> Option<(SogMeta, Vec<RawSplat>)> {
    let text = std::fs::read_to_string(folder.join("meta.json")).ok()?;
    let meta = SogMeta::parse(&text).ok()?;
    let plane = |name: &str| decode_webp(&folder.join(name)).map(|(rgba, ..)| rgba);
    let means_l = plane(&meta.means_files[0])?;
    let means_u = plane(&meta.means_files[1])?;
    let quats = plane(&meta.quats_file)?;
    let scales = plane(&meta.scales_file)?;
    let sh0 = plane(&meta.sh0_file)?;
    let splats = SogLoader {
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
    .ok()?;
    Some((meta, splats))
}
