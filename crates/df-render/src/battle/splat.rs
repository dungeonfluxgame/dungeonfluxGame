//! Gaussian splat data: loaders, the baked cloud, the procedural test ground and the depth sort.
//!
//! Loaders sit behind one trait, [`SplatLoader`], and all produce [`RawSplat`]s (position,
//! linear scale, rotation, straight-alpha colour). [`SplatCloud::bake`] then applies the scene
//! transform, precomputes each splat's world-space 3D covariance and orders the splats by
//! importance so a prefix of the cloud is a valid level of detail: the frame-time budget just
//! lowers how many it draws. The sorter orders by quantised view depth with a counting sort.
//!
//! The SOG decoding follows PlayCanvas's `SogBundle` reader (MIT): positions are stored as 16-bit
//! values in two planes with a log transform, rotations as "smallest three" in RGB with the
//! index of the largest component in alpha, and scales and colours as codebook (version 2) or
//! min/max (version 1) indexes. See `THIRD_PARTY_NOTICES.md`.

use serde::Deserialize;

use super::math::{Quat, Vec3};

/// Hard ceiling on a scene's splat count; larger inputs are refused, not truncated silently.
pub const MAX_SPLATS: usize = 4_000_000;
const SH_C0: f32 = 0.282_094_8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SplatError {
    /// The `.splat` byte length is not a multiple of 32.
    BadLength,
    TooMany,
    NonFinite,
    MetaInvalid(&'static str),
    /// A SOG plane is smaller than the splat count needs.
    PlaneSize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawSplat {
    pub position: Vec3,
    /// Linear (not log) per-axis standard deviation in metres.
    pub scale: Vec3,
    pub rotation: Quat,
    /// Straight (non-premultiplied) RGBA, alpha is opacity.
    pub colour: [u8; 4],
}

pub trait SplatLoader {
    fn load(&self) -> Result<Vec<RawSplat>, SplatError>;
}

/// The antimatter15 `.splat` layout: per splat 3 f32 position, 3 f32 scale, 4 u8 RGBA and
/// 4 u8 rotation (w, x, y, z as `q * 128 + 128`), 32 bytes in all.
pub struct SplatFileLoader<'a> {
    pub bytes: &'a [u8],
}

pub const SPLAT_RECORD_BYTES: usize = 32;

impl SplatLoader for SplatFileLoader<'_> {
    fn load(&self) -> Result<Vec<RawSplat>, SplatError> {
        if !self.bytes.len().is_multiple_of(SPLAT_RECORD_BYTES) {
            return Err(SplatError::BadLength);
        }
        let count = self.bytes.len() / SPLAT_RECORD_BYTES;
        if count > MAX_SPLATS {
            return Err(SplatError::TooMany);
        }
        let mut splats = Vec::with_capacity(count);
        for record in self.bytes.as_chunks::<SPLAT_RECORD_BYTES>().0 {
            let float = |index: usize| {
                f32::from_le_bytes([
                    record[index * 4],
                    record[index * 4 + 1],
                    record[index * 4 + 2],
                    record[index * 4 + 3],
                ])
            };
            let position = Vec3::new(float(0), float(1), float(2));
            let scale = Vec3::new(float(3), float(4), float(5));
            if !position.is_finite() || !scale.is_finite() {
                return Err(SplatError::NonFinite);
            }
            let unit = |byte: u8| (byte as f32 - 128.0) / 128.0;
            splats.push(RawSplat {
                position,
                scale,
                rotation: Quat::new(
                    unit(record[29]),
                    unit(record[30]),
                    unit(record[31]),
                    unit(record[28]),
                )
                .normalized(),
                colour: [record[24], record[25], record[26], record[27]],
            });
        }
        Ok(splats)
    }
}

/// Encodes splats into the `.splat` layout; used by tests and tooling to round-trip.
pub fn encode_splat_file(splats: &[RawSplat]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(splats.len() * SPLAT_RECORD_BYTES);
    for splat in splats {
        for value in [
            splat.position.x,
            splat.position.y,
            splat.position.z,
            splat.scale.x,
            splat.scale.y,
            splat.scale.z,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&splat.colour);
        let q = splat.rotation;
        for component in [q.w, q.x, q.y, q.z] {
            bytes.push((component * 128.0 + 128.0).round().clamp(0.0, 255.0) as u8);
        }
    }
    bytes
}

#[derive(Deserialize)]
struct SogCodebookOrRange {
    #[serde(default)]
    codebook: Option<Vec<f32>>,
    #[serde(default)]
    mins: Option<[f32; 3]>,
    #[serde(default)]
    maxs: Option<[f32; 3]>,
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Deserialize)]
struct SogFiles {
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Deserialize)]
struct SogMeans {
    mins: [f32; 3],
    maxs: [f32; 3],
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Deserialize)]
struct SogMetaFile {
    version: u32,
    count: usize,
    means: SogMeans,
    scales: SogCodebookOrRange,
    quats: SogFiles,
    sh0: SogCodebookOrRange,
}

/// Parsed `meta.json` of a PlayCanvas SOG bundle.
#[derive(Clone, Debug, PartialEq)]
pub struct SogMeta {
    pub version: u32,
    pub count: usize,
    means: ([f32; 3], [f32; 3]),
    scales: Quantiser,
    colours: Quantiser,
    pub means_files: [String; 2],
    pub quats_file: String,
    pub scales_file: String,
    pub sh0_file: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Quantiser {
    Codebook(Vec<f32>),
    Range([f32; 3], [f32; 3]),
}

impl Quantiser {
    fn value(&self, channel: usize, byte: u8) -> f32 {
        match self {
            Self::Codebook(values) => values.get(byte as usize).copied().unwrap_or(0.0),
            Self::Range(mins, maxs) => {
                mins[channel] + (maxs[channel] - mins[channel]) * byte as f32 / 255.0
            }
        }
    }
}

fn quantiser(
    source: SogCodebookOrRange,
    version: u32,
) -> Result<(Quantiser, Vec<String>), SplatError> {
    let quantiser = match (version, source.codebook, source.mins, source.maxs) {
        (2.., Some(codebook), _, _) if (1..=256).contains(&codebook.len()) => {
            Quantiser::Codebook(codebook)
        }
        (_, None, Some(mins), Some(maxs)) => Quantiser::Range(mins, maxs),
        _ => return Err(SplatError::MetaInvalid("quantiser")),
    };
    Ok((quantiser, source.files))
}

impl SogMeta {
    /// The log-space range the 16-bit positions are spread over, per axis (`mins`, `maxs`).
    pub fn means_range(&self) -> ([f32; 3], [f32; 3]) {
        self.means
    }

    /// The scale and colour codebooks of a version 2 file, padded to 256 entries; `None` for version 1, whose
    /// min/max quantisation the GPU decoder does not implement.
    pub fn codebooks(&self) -> Option<([f32; 256], [f32; 256])> {
        // A small chunk may have fewer than 256 distinct values; the rest of the table is unused.
        let table = |quantiser: &Quantiser| match quantiser {
            Quantiser::Codebook(values) => {
                let mut padded = [0.0f32; 256];
                padded.get_mut(..values.len())?.copy_from_slice(values);
                Some(padded)
            }
            Quantiser::Range(..) => None,
        };
        Some((table(&self.scales)?, table(&self.colours)?))
    }

    pub fn parse(json: &str) -> Result<Self, SplatError> {
        let meta: SogMetaFile =
            serde_json::from_str(json).map_err(|_| SplatError::MetaInvalid("json"))?;
        if !(1..=2).contains(&meta.version) {
            return Err(SplatError::MetaInvalid("version"));
        }
        if meta.count == 0 || meta.count > MAX_SPLATS {
            return Err(SplatError::MetaInvalid("count"));
        }
        let [means_l, means_u] = <[String; 2]>::try_from(meta.means.files)
            .map_err(|_| SplatError::MetaInvalid("means files"))?;
        let [quats] = <[String; 1]>::try_from(meta.quats.files)
            .map_err(|_| SplatError::MetaInvalid("quats files"))?;
        let (scales, scale_files) = quantiser(meta.scales, meta.version)?;
        let (colours, colour_files) = quantiser(meta.sh0, meta.version)?;
        let [scales_file] = <[String; 1]>::try_from(scale_files)
            .map_err(|_| SplatError::MetaInvalid("scales files"))?;
        let [sh0_file] = <[String; 1]>::try_from(colour_files)
            .map_err(|_| SplatError::MetaInvalid("sh0 files"))?;
        for name in [&means_l, &means_u, &quats, &scales_file, &sh0_file] {
            let plain = !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
                && !name.contains("..");
            if !plain {
                return Err(SplatError::MetaInvalid("file name"));
            }
        }
        Ok(Self {
            version: meta.version,
            count: meta.count,
            means: (meta.means.mins, meta.means.maxs),
            scales,
            colours,
            means_files: [means_l, means_u],
            quats_file: quats,
            scales_file,
            sh0_file,
        })
    }
}

/// The five decoded RGBA planes of a SOG bundle, each at least `count * 4` bytes.
pub struct SogPlanes<'a> {
    pub means_l: &'a [u8],
    pub means_u: &'a [u8],
    pub quats: &'a [u8],
    pub scales: &'a [u8],
    pub sh0: &'a [u8],
}

pub struct SogLoader<'a> {
    pub meta: &'a SogMeta,
    pub planes: SogPlanes<'a>,
}

fn unlog(value: f32) -> f32 {
    value.signum() * (value.abs().exp() - 1.0)
}

impl SplatLoader for SogLoader<'_> {
    fn load(&self) -> Result<Vec<RawSplat>, SplatError> {
        let count = self.meta.count;
        let planes = &self.planes;
        for plane in [
            planes.means_l,
            planes.means_u,
            planes.quats,
            planes.scales,
            planes.sh0,
        ] {
            if plane.len() < count * 4 {
                return Err(SplatError::PlaneSize);
            }
        }
        let (mins, maxs) = self.meta.means;
        let mut splats = Vec::with_capacity(count);
        for index in 0..count {
            let at = index * 4;
            let mut position = [0.0f32; 3];
            for (axis, slot) in position.iter_mut().enumerate() {
                let packed =
                    ((planes.means_u[at + axis] as u32) << 8) | planes.means_l[at + axis] as u32;
                let fraction = packed as f32 / 65535.0;
                *slot = unlog(mins[axis] + (maxs[axis] - mins[axis]) * fraction);
            }
            let scale = Vec3::new(
                self.meta.scales.value(0, planes.scales[at]).exp(),
                self.meta.scales.value(1, planes.scales[at + 1]).exp(),
                self.meta.scales.value(2, planes.scales[at + 2]).exp(),
            );
            let colour = |channel: usize| {
                let value =
                    0.5 + self.meta.colours.value(channel, planes.sh0[at + channel]) * SH_C0;
                (value.clamp(0.0, 1.0) * 255.0).round() as u8
            };
            let position = Vec3::new(position[0], position[1], position[2]);
            if !position.is_finite() || !scale.is_finite() {
                return Err(SplatError::NonFinite);
            }
            splats.push(RawSplat {
                position,
                scale,
                rotation: decode_rotation(&planes.quats[at..at + 4]),
                colour: [colour(0), colour(1), colour(2), planes.sh0[at + 3]],
            });
        }
        Ok(splats)
    }
}

/// "Smallest three" quaternion: RGB hold three components scaled by sqrt(2); alpha minus 252 is
/// the index (x, y, z, w) of the largest component, recovered from unit length.
fn decode_rotation(bytes: &[u8]) -> Quat {
    let part = |byte: u8| (byte as f32 / 255.0 - 0.5) * std::f32::consts::SQRT_2;
    let (a, b, c) = (part(bytes[0]), part(bytes[1]), part(bytes[2]));
    let largest = (1.0 - (a * a + b * b + c * c)).max(0.0).sqrt();
    match bytes[3].wrapping_sub(252) {
        0 => Quat::new(a, b, c, largest),
        1 => Quat::new(largest, b, c, a),
        2 => Quat::new(b, largest, c, a),
        3 => Quat::new(b, c, largest, a),
        _ => Quat::IDENTITY,
    }
    .normalized()
}

/// Deterministic procedural test ground: an uneven dark earth surface with scattered rocks,
/// standing in for a real scan so the splat pass can be checked without external data. The
/// middle of the battlefield is kept nearly flat so figures stand on it.
pub struct ProceduralGround {
    pub seed: u32,
    pub splats: usize,
    pub half_extent_m: f32,
}

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / 16_777_216.0
    }
}

fn hash_noise(x: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393) ^ (z as u32).wrapping_mul(668_265_263) ^ seed;
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

fn smooth_noise(x: f32, z: f32, seed: u32) -> f32 {
    let (xi, zi) = (x.floor(), z.floor());
    let (fx, fz) = (x - xi, z - zi);
    let (sx, sz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let n = |dx: i32, dz: i32| hash_noise(xi as i32 + dx, zi as i32 + dz, seed);
    let top = n(0, 0) + (n(1, 0) - n(0, 0)) * sx;
    let bottom = n(0, 1) + (n(1, 1) - n(0, 1)) * sx;
    top + (bottom - top) * sz
}

/// Height of the procedural ground at a point, so tests can check figures would stand on it.
pub fn procedural_height(x: f32, z: f32) -> f32 {
    let radius = (x * x + z * z).sqrt();
    let flatten = ((radius - 5.0) / 6.0).clamp(0.0, 1.0);
    let bumps =
        smooth_noise(x * 0.35, z * 0.35, 7) * 0.9 + smooth_noise(x * 1.3, z * 1.3, 11) * 0.12;
    (bumps - 0.45) * flatten * 0.8 + smooth_noise(x * 4.0, z * 4.0, 3) * 0.015
}

impl SplatLoader for ProceduralGround {
    fn load(&self) -> Result<Vec<RawSplat>, SplatError> {
        if self.splats == 0 || self.splats > MAX_SPLATS || !self.half_extent_m.is_finite() {
            return Err(SplatError::TooMany);
        }
        let mut random = Lcg(self.seed ^ 0x9e37_79b9);
        let rocks = 18;
        let per_rock = self.splats / 8 / rocks;
        let ground_splats = self.splats - per_rock * rocks;
        let mut splats = Vec::with_capacity(self.splats);
        for _ in 0..ground_splats {
            let x = (random.next() * 2.0 - 1.0) * self.half_extent_m;
            let z = (random.next() * 2.0 - 1.0) * self.half_extent_m;
            let y = procedural_height(x, z);
            let grain = smooth_noise(x * 2.0, z * 2.0, 5);
            let tone = 0.22 + grain * 0.2 + random.next() * 0.06;
            let warm = smooth_noise(x * 0.5 + 40.0, z * 0.5, 9);
            let colour = [
                (tone * (0.8 + warm * 0.5) * 255.0) as u8,
                (tone * (0.78 + warm * 0.2) * 255.0) as u8,
                (tone * (0.82 - warm * 0.1) * 255.0) as u8,
                235,
            ];
            let sigma = 0.032 + random.next() * 0.02;
            splats.push(RawSplat {
                position: Vec3::new(x, y, z),
                scale: Vec3::new(sigma, sigma * 0.18, sigma),
                rotation: Quat::from_axis_angle_y(random.next() * std::f32::consts::TAU),
                colour,
            });
        }
        for rock in 0..rocks {
            let angle = rock as f32 / rocks as f32 * std::f32::consts::TAU + random.next();
            let distance = 8.0 + random.next() * (self.half_extent_m - 9.0).max(1.0);
            let (cx, cz) = (angle.cos() * distance, angle.sin() * distance);
            let radius = 0.4 + random.next() * 0.8;
            let base = procedural_height(cx, cz);
            for _ in 0..per_rock {
                let (u, v) = (
                    random.next() * 2.0 - 1.0,
                    random.next() * std::f32::consts::TAU,
                );
                let ring = (1.0 - u * u).sqrt();
                let normal = Vec3::new(ring * v.cos(), u.abs(), ring * v.sin());
                let lump = 1.0
                    + (smooth_noise(normal.x * 3.0 + rock as f32, normal.z * 3.0, 13) - 0.5) * 0.5;
                let position = Vec3::new(cx, base - radius * 0.15, cz) + normal * (radius * lump);
                let shade = 0.1
                    + smooth_noise(position.x * 6.0, position.y * 6.0 + position.z * 6.0, 17) * 0.2;
                splats.push(RawSplat {
                    position,
                    scale: Vec3::new(0.05, 0.05, 0.05),
                    rotation: Quat::IDENTITY,
                    colour: [
                        (shade * 0.95 * 255.0) as u8,
                        (shade * 0.97 * 255.0) as u8,
                        (shade * 255.0) as u8,
                        240,
                    ],
                });
            }
        }
        Ok(splats)
    }
}

/// Placement of a splat scene on the battlefield floor (the earlier build's `transform`), plus
/// the orientation that turns a scan's own axes upright. A scan point goes through `orient`
/// (a scan is often upside down or lying on its side), then the yaw about the vertical axis, then
/// the uniform scale, then the offsets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplatTransform {
    pub scale: f32,
    pub offset_y: f32,
    pub rot_y_deg: f32,
    pub translate: Vec3,
    pub orient: Quat,
}

impl SplatTransform {
    pub const IDENTITY: Self = Self {
        scale: 1.0,
        offset_y: 0.0,
        rot_y_deg: 0.0,
        translate: Vec3::ZERO,
        orient: Quat::IDENTITY,
    };

    /// The scan-to-world rotation (orientation then yaw).
    pub fn rotation(&self) -> Quat {
        Quat::from_axis_angle_y(self.rot_y_deg.to_radians())
            .compose(self.orient)
            .normalized()
    }

    fn offset(&self) -> Vec3 {
        Vec3::new(0.0, self.offset_y, 0.0) + self.translate
    }

    pub fn to_world(&self, scan: Vec3) -> Vec3 {
        self.rotation().rotate(scan * self.scale) + self.offset()
    }

    pub fn to_scan(&self, world: Vec3) -> Vec3 {
        let q = self.rotation();
        let inverse = Quat::new(-q.x, -q.y, -q.z, q.w);
        inverse.rotate(world - self.offset()) * (1.0 / self.scale)
    }

    /// Row-major 3x3 (rotation times scale) and the translation: `world = m * scan + t`.
    pub fn affine(&self) -> ([[f32; 3]; 3], Vec3) {
        let mut m = self.rotation().to_matrix3();
        for row in &mut m {
            for value in row.iter_mut() {
                *value *= self.scale;
            }
        }
        (m, self.offset())
    }
}

/// Rotation about X, then Y, then Z by the given degrees: the way a scene file states the
/// orientation of a scan.
pub fn orientation_from_euler_deg(degrees: [f32; 3]) -> Quat {
    let about = |axis: Vec3, degrees: f32| {
        let half = degrees.to_radians() * 0.5;
        Quat::new(
            axis.x * half.sin(),
            axis.y * half.sin(),
            axis.z * half.sin(),
            half.cos(),
        )
    };
    about(Vec3::new(0.0, 0.0, 1.0), degrees[2])
        .compose(about(Vec3::new(0.0, 1.0, 0.0), degrees[1]))
        .compose(about(Vec3::new(1.0, 0.0, 0.0), degrees[0]))
}

/// A baked splat ready for the GPU: world position, straight RGBA, world covariance
/// (`xx, xy, xz, yy, yz, zz`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Splat {
    pub centre: Vec3,
    pub colour: [u8; 4],
    pub covariance: [f32; 6],
}

pub const GPU_SPLAT_FLOATS: usize = 12;

#[derive(Clone, Debug, Default)]
pub struct SplatCloud {
    splats: Vec<Splat>,
}

fn covariance(rotation: Quat, scale: Vec3) -> [f32; 6] {
    let r = rotation.to_matrix3();
    let s = [scale.x * scale.x, scale.y * scale.y, scale.z * scale.z];
    let at = |i: usize, j: usize| (0..3).map(|k| r[i][k] * s[k] * r[j][k]).sum::<f32>();
    [at(0, 0), at(0, 1), at(0, 2), at(1, 1), at(1, 2), at(2, 2)]
}

impl SplatCloud {
    /// Applies `transform`, precomputes covariances and orders by importance (opacity times
    /// size), most important first. Splats with non-finite values are dropped, and the count
    /// is reported so a scan with bad data is visible rather than silently shorter.
    pub fn bake(raw: &[RawSplat], transform: SplatTransform) -> (Self, usize) {
        let turn = transform.rotation();
        let mut dropped = 0;
        let mut ranked: Vec<(f32, Splat)> = Vec::with_capacity(raw.len());
        for splat in raw {
            if !splat.position.is_finite() || !splat.scale.is_finite() {
                dropped += 1;
                continue;
            }
            let scale = splat.scale * transform.scale;
            let rotation = turn.compose(splat.rotation).normalized();
            let centre = transform.to_world(splat.position);
            let size = (scale.x * scale.y * scale.z).abs().cbrt();
            ranked.push((
                splat.colour[3] as f32 * size,
                Splat {
                    centre,
                    colour: splat.colour,
                    covariance: covariance(rotation, scale),
                },
            ));
        }
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        (
            Self {
                splats: ranked.into_iter().map(|(_, splat)| splat).collect(),
            },
            dropped,
        )
    }

    pub fn len(&self) -> usize {
        self.splats.len()
    }

    pub fn is_empty(&self) -> bool {
        self.splats.is_empty()
    }

    pub fn splats(&self) -> &[Splat] {
        &self.splats
    }

    /// Interleaved GPU records: `centre.xyz, colour (RGBA8 bits), cov xx xy xz yy, cov yz zz, 0, 0`.
    pub fn gpu_floats(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.splats.len() * GPU_SPLAT_FLOATS);
        for splat in &self.splats {
            let colour = u32::from_le_bytes(splat.colour);
            let c = &splat.covariance;
            out.extend_from_slice(&[
                splat.centre.x,
                splat.centre.y,
                splat.centre.z,
                f32::from_bits(colour),
                c[0],
                c[1],
                c[2],
                c[3],
                c[4],
                c[5],
                0.0,
                0.0,
            ]);
        }
        out
    }
}

/// Floats per GPU splat record: position (3), colour RGBA8 (1), rotation as two packed snorm16
/// pairs (2) and scale as two packed f16 pairs (2): eight 32-bit words, 32 bytes.
pub const RECORD_WORDS: usize = 8;

/// IEEE half-precision bits of `value` (round to nearest, saturating at the largest finite half).
pub fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let magnitude = value.abs();
    if magnitude.is_nan() {
        return sign | 0x7e00;
    }
    if magnitude >= 65504.0 {
        return sign | 0x7bff;
    }
    if magnitude < 6.103_515_6e-5 {
        // Subnormal halves are multiples of 2^-24.
        return sign | (magnitude * 16_777_216.0).round() as u16;
    }
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = (bits & 0x007f_ffff) as f32 / 8_388_608.0;
    let half = ((exponent as u32) << 10) + (mantissa * 1024.0).round() as u32;
    sign | half as u16
}

fn snorm16_bits(value: f32) -> u32 {
    u32::from((value.clamp(-1.0, 1.0) * 32767.0).round() as i16 as u16)
}

/// Packs one splat, already placed in the world, into the eight words the splat shaders read.
pub fn pack_record(
    centre: Vec3,
    scale: Vec3,
    rotation: Quat,
    colour: [u8; 4],
) -> [u32; RECORD_WORDS] {
    let q = rotation.normalized();
    let half =
        |a: f32, b: f32| u32::from(f32_to_f16_bits(a)) | (u32::from(f32_to_f16_bits(b)) << 16);
    [
        centre.x.to_bits(),
        centre.y.to_bits(),
        centre.z.to_bits(),
        u32::from_le_bytes(colour),
        snorm16_bits(q.x) | (snorm16_bits(q.y) << 16),
        snorm16_bits(q.z) | (snorm16_bits(q.w) << 16),
        half(scale.x, scale.y),
        half(scale.z, 0.0),
    ]
}

/// Places `raw` splats with `transform` and packs them into GPU records. Splats with non-finite
/// values are dropped and counted, as in [`SplatCloud::bake`].
pub fn pack_records(raw: &[RawSplat], transform: SplatTransform) -> (Vec<u32>, usize) {
    let turn = transform.rotation();
    let mut words = Vec::with_capacity(raw.len() * RECORD_WORDS);
    let mut dropped = 0;
    for splat in raw {
        if !splat.position.is_finite() || !splat.scale.is_finite() {
            dropped += 1;
            continue;
        }
        words.extend_from_slice(&pack_record(
            transform.to_world(splat.position),
            splat.scale * transform.scale,
            turn.compose(splat.rotation),
            splat.colour,
        ));
    }
    (words, dropped)
}

const SORT_BUCKETS: usize = 1 << 16;

/// The depth sort key the GPU projection pass writes (`project_splat` in `splat_shaders.rs`):
/// far splats get small keys so ascending key order is back to front, and the square-root
/// spacing keeps the finest steps near the camera. Kept here, pure, so the mapping is tested.
pub fn depth_sort_key(depth_m: f32, range_m: f32) -> u32 {
    let t = (depth_m / range_m).clamp(0.0, 1.0);
    65535 - (t.sqrt() * 65535.0) as u32
}

/// What the GPU's counting sort produces from keys: indexes grouped by ascending key. Within one
/// key the GPU's order is arbitrary (atomic slot claims), which cannot be seen because splats in
/// one key are within a few millimetres of each other in depth; this stable version is the
/// reference the tests compare against.
pub fn counting_order(keys: &[u32]) -> Vec<u32> {
    let mut counts = vec![0u32; SORT_BUCKETS + 1];
    for key in keys {
        counts[*key as usize + 1] += 1;
    }
    for bin in 1..counts.len() {
        counts[bin] += counts[bin - 1];
    }
    let mut order = vec![0u32; keys.len()];
    for (index, key) in keys.iter().enumerate() {
        let slot = &mut counts[*key as usize];
        order[*slot as usize] = index as u32;
        *slot += 1;
    }
    order
}

/// Orders splats back to front by quantised view depth with a counting sort, and remembers the
/// camera of the last sort so it re-sorts only when the view has moved enough to change the order.
pub struct SplatSorter {
    order: Vec<u32>,
    keys: Vec<u16>,
    counts: Vec<u32>,
    last: Option<(Vec3, Vec3)>,
    pub eye_threshold_m: f32,
    pub direction_threshold_cos: f32,
}

impl Default for SplatSorter {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            keys: Vec::new(),
            counts: vec![0; SORT_BUCKETS],
            last: None,
            eye_threshold_m: 0.2,
            // About 0.8 degrees.
            direction_threshold_cos: 0.9999,
        }
    }
}

impl SplatSorter {
    pub fn order(&self) -> &[u32] {
        &self.order
    }

    /// Whether the view moved enough since the last sort to need another.
    pub fn needs_sort(&self, eye: Vec3, forward: Vec3) -> bool {
        match self.last {
            None => true,
            Some((last_eye, last_forward)) => {
                (eye - last_eye).length() > self.eye_threshold_m
                    || forward.dot(last_forward) < self.direction_threshold_cos
            }
        }
    }

    /// Forces the next [`Self::needs_sort`] to be true (after the splat set or budget changed).
    pub fn invalidate(&mut self) {
        self.last = None;
    }

    /// Sorts the first `active` splats far to near along `forward`, dropping those not in front
    /// of the camera plane. Returns the order (indexes into `splats`).
    pub fn sort(&mut self, splats: &[Splat], active: usize, eye: Vec3, forward: Vec3) -> &[u32] {
        let active = active.min(splats.len());
        let depth = |splat: &Splat| (splat.centre - eye).dot(forward);
        let (mut near, mut far) = (f32::MAX, f32::MIN);
        for splat in &splats[..active] {
            let d = depth(splat);
            if d > 0.0 {
                near = near.min(d);
                far = far.max(d);
            }
        }
        self.order.clear();
        self.keys.clear();
        self.last = Some((eye, forward));
        if near > far {
            return &self.order;
        }
        let range = (far - near).max(1e-6);
        self.counts.iter_mut().for_each(|count| *count = 0);
        let mut visible: Vec<u32> = Vec::with_capacity(active);
        for (index, splat) in splats[..active].iter().enumerate() {
            let d = depth(splat);
            if d <= 0.0 {
                continue;
            }
            // Far splats get small keys, so ascending key order is back to front.
            let key = (((far - d) / range) * (SORT_BUCKETS as f32 - 1.0)) as u16;
            self.counts[key as usize] += 1;
            self.keys.push(key);
            visible.push(index as u32);
        }
        let mut running = 0u32;
        for count in self.counts.iter_mut() {
            let here = *count;
            *count = running;
            running += here;
        }
        self.order.resize(visible.len(), 0);
        for (slot, index) in visible.iter().enumerate() {
            let key = self.keys[slot] as usize;
            let target = self.counts[key] as usize;
            self.counts[key] += 1;
            self.order[target] = *index;
        }
        &self.order
    }
}

/// The reference the sorter is checked against: a stable sort on the same quantised key.
#[cfg(test)]
fn brute_force_order(splats: &[Splat], eye: Vec3, forward: Vec3) -> Vec<u32> {
    let depth = |splat: &Splat| (splat.centre - eye).dot(forward);
    let in_front: Vec<(u32, f32)> = splats
        .iter()
        .enumerate()
        .map(|(index, splat)| (index as u32, depth(splat)))
        .filter(|(_, d)| *d > 0.0)
        .collect();
    let far = in_front.iter().map(|(_, d)| *d).fold(f32::MIN, f32::max);
    let near = in_front.iter().map(|(_, d)| *d).fold(f32::MAX, f32::min);
    let range = (far - near).max(1e-6);
    let mut keyed: Vec<(u16, u32)> = in_front
        .iter()
        .map(|(index, d)| {
            (
                (((far - d) / range) * (SORT_BUCKETS as f32 - 1.0)) as u16,
                *index,
            )
        })
        .collect();
    keyed.sort_by_key(|(key, _)| *key);
    keyed.into_iter().map(|(_, index)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5 + 1e-4 * b.abs()
    }

    fn raw(x: f32, y: f32, z: f32) -> RawSplat {
        RawSplat {
            position: Vec3::new(x, y, z),
            scale: Vec3::new(0.1, 0.2, 0.3),
            rotation: Quat::from_axis_angle_y(0.7),
            colour: [10, 120, 250, 200],
        }
    }

    #[test]
    fn a_tiny_synthetic_splat_file_round_trips() {
        let original = vec![raw(1.0, 2.0, 3.0), raw(-4.5, 0.25, 9.0)];
        let bytes = encode_splat_file(&original);
        assert_eq!(bytes.len(), 64);
        let loaded = SplatFileLoader { bytes: &bytes }.load().unwrap();
        assert_eq!(loaded.len(), 2);
        for (a, b) in original.iter().zip(&loaded) {
            assert_eq!(a.position, b.position);
            assert_eq!(a.scale, b.scale);
            assert_eq!(a.colour, b.colour);
            // Rotation is quantised to a byte per component.
            assert!((a.rotation.y - b.rotation.y).abs() < 0.02);
            assert!((a.rotation.w - b.rotation.w).abs() < 0.02);
        }
    }

    #[test]
    fn splat_files_with_a_bad_length_or_non_finite_data_are_refused() {
        assert_eq!(
            SplatFileLoader { bytes: &[0; 33] }.load().unwrap_err(),
            SplatError::BadLength
        );
        let mut bytes = encode_splat_file(&[raw(0.0, 0.0, 0.0)]);
        bytes[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            SplatFileLoader { bytes: &bytes }.load().unwrap_err(),
            SplatError::NonFinite
        );
        assert!(SplatFileLoader { bytes: &[] }.load().unwrap().is_empty());
    }

    const SOG_V2: &str = r#"{
        "version": 2, "count": 2,
        "means": {"mins": [-1.0, -1.0, -1.0], "maxs": [1.0, 1.0, 1.0], "files": ["means_l.webp", "means_u.webp"]},
        "scales": {"codebook": [], "files": ["scales.webp"]},
        "quats": {"files": ["quats.webp"]},
        "sh0": {"codebook": [], "files": ["sh0.webp"]}
    }"#;

    fn sog_json(version: u32) -> String {
        let book: Vec<String> = (0..256)
            .map(|i| format!("{}", -4.0 + i as f32 * 0.02))
            .collect();
        let book = format!("[{}]", book.join(","));
        if version == 2 {
            SOG_V2.replace("[]", &book)
        } else {
            r#"{"version":1,"count":2,
                "means":{"mins":[-1,-1,-1],"maxs":[1,1,1],"files":["means_l.webp","means_u.webp"]},
                "scales":{"mins":[-4,-4,-4],"maxs":[0,0,0],"files":["scales.webp"]},
                "quats":{"files":["quats.webp"]},
                "sh0":{"mins":[-2,-2,-2],"maxs":[2,2,2],"files":["sh0.webp"]}}"#
                .to_owned()
        }
    }

    #[test]
    fn sog_meta_parses_both_versions_and_names_the_planes() {
        let v2 = SogMeta::parse(&sog_json(2)).unwrap();
        assert_eq!((v2.version, v2.count), (2, 2));
        assert_eq!(
            v2.means_files,
            ["means_l.webp".to_owned(), "means_u.webp".to_owned()]
        );
        assert_eq!(v2.quats_file, "quats.webp");
        let v1 = SogMeta::parse(&sog_json(1)).unwrap();
        assert_eq!(v1.version, 1);
    }

    #[test]
    fn sog_meta_rejects_bad_versions_counts_and_path_like_file_names() {
        assert!(SogMeta::parse("not json").is_err());
        assert_eq!(
            SogMeta::parse(&sog_json(2).replace("\"version\": 2", "\"version\": 3")).unwrap_err(),
            SplatError::MetaInvalid("version")
        );
        assert_eq!(
            SogMeta::parse(&sog_json(2).replace("\"count\": 2", "\"count\": 0")).unwrap_err(),
            SplatError::MetaInvalid("count")
        );
        assert_eq!(
            SogMeta::parse(&sog_json(2).replace("quats.webp", "../quats.webp")).unwrap_err(),
            SplatError::MetaInvalid("file name")
        );
        // A version 2 file needs a 256-entry codebook.
        assert!(SogMeta::parse(SOG_V2).is_err());
    }

    #[test]
    fn sog_planes_decode_positions_scales_colours_and_rotations() {
        let meta = SogMeta::parse(&sog_json(2)).unwrap();
        // x: 16-bit value 0xFFFF -> max (1.0) -> unlog; y: 0 -> min; z: mid.
        let means_l = [0xFF, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00];
        let means_u = [0xFF, 0x00, 0x80, 0xFF, 0x80, 0x80, 0x00, 0x00];
        // Largest component is w (index 3 -> alpha 255): identity is x=y=z=0 -> 0.5 each.
        let quats = [128, 128, 128, 252, 128, 128, 128, 252];
        let scales = [200, 200, 200, 255, 0, 0, 0, 255];
        let sh0 = [128, 128, 128, 255, 255, 255, 255, 128];
        let loaded = SogLoader {
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
        .unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(close(loaded[0].position.x, 1.0f32.exp() - 1.0));
        assert!(close(loaded[0].position.y, -(1.0f32.exp() - 1.0)));
        assert!(loaded[0].position.z.abs() < 0.01);
        assert!(close(loaded[0].scale.x, (-4.0f32 + 200.0 * 0.02).exp()));
        assert!(loaded[0].rotation.w > 0.99);
        assert_eq!(loaded[0].colour[3], 255);
        assert_eq!(loaded[1].colour[3], 128);
        assert!(
            loaded[1].colour[0] > 128,
            "bright colour plane gives bright colour"
        );
    }

    #[test]
    fn sog_planes_that_are_too_small_are_refused() {
        let meta = SogMeta::parse(&sog_json(2)).unwrap();
        let small = [0u8; 4];
        let full = [0u8; 8];
        let result = SogLoader {
            meta: &meta,
            planes: SogPlanes {
                means_l: &full,
                means_u: &full,
                quats: &full,
                scales: &small,
                sh0: &full,
            },
        }
        .load();
        assert_eq!(result.unwrap_err(), SplatError::PlaneSize);
    }

    #[test]
    fn sog_rotation_decoding_handles_each_largest_component_index() {
        let rotations = [
            Quat::new(0.5, 0.5, 0.5, 0.5),
            Quat::new(0.1, 0.2, 0.3, 0.9273618).normalized(),
            Quat::new(0.9, 0.2, -0.3, 0.2).normalized(),
            Quat::new(0.1, -0.9, 0.3, 0.2).normalized(),
            Quat::new(-0.1, 0.2, 0.9, 0.3).normalized(),
        ];
        for q in rotations {
            // Component order x, y, z, w; the stored code orders w, x, y, z.
            let components = [q.w, q.x, q.y, q.z];
            let (largest, _) = components
                .iter()
                .enumerate()
                .fold((0, f32::MIN), |best, (i, v)| {
                    if v.abs() > best.1 { (i, v.abs()) } else { best }
                });
            let sign = components[largest].signum();
            let others: Vec<f32> = (0..4)
                .filter(|i| *i != largest)
                .map(|i| components[i] * sign)
                .collect();
            let byte = |v: f32| ((v / std::f32::consts::SQRT_2 + 0.5) * 255.0).round() as u8;
            let bytes = [
                byte(others[0]),
                byte(others[1]),
                byte(others[2]),
                252 + largest as u8,
            ];
            let decoded = decode_rotation(&bytes);
            let dot = decoded.x * q.x + decoded.y * q.y + decoded.z * q.z + decoded.w * q.w;
            assert!(dot.abs() > 0.995, "largest {largest}: dot {dot}");
        }
    }

    #[test]
    fn baking_applies_the_scene_transform_and_orders_by_importance() {
        let big = RawSplat {
            scale: Vec3::new(0.5, 0.5, 0.5),
            ..raw(1.0, 0.0, 0.0)
        };
        let small = RawSplat {
            scale: Vec3::new(0.01, 0.01, 0.01),
            ..raw(2.0, 0.0, 0.0)
        };
        let (cloud, dropped) = SplatCloud::bake(
            &[small, big],
            SplatTransform {
                scale: 2.0,
                offset_y: 1.0,
                rot_y_deg: 90.0,
                translate: Vec3::new(0.0, 0.0, 10.0),
                orient: Quat::IDENTITY,
            },
        );
        assert_eq!(dropped, 0);
        // The big splat ranks first and x=1 scaled by 2 and turned 90 degrees about Y lands on -Z.
        let first = cloud.splats()[0].centre;
        assert!(close(first.x, 0.0) && close(first.y, 1.0) && close(first.z, 10.0 - 2.0));
        // Scale doubled: variance quadrupled.
        let covariance = cloud.splats()[0].covariance;
        let trace = covariance[0] + covariance[3] + covariance[5];
        assert!(close(trace, 3.0), "trace {trace}");
        assert_eq!(cloud.gpu_floats().len(), 2 * GPU_SPLAT_FLOATS);
    }

    #[test]
    fn baking_drops_non_finite_splats_and_counts_them() {
        let bad = RawSplat {
            position: Vec3::new(f32::INFINITY, 0.0, 0.0),
            ..raw(0.0, 0.0, 0.0)
        };
        let (cloud, dropped) =
            SplatCloud::bake(&[bad, raw(0.0, 0.0, 0.0)], SplatTransform::IDENTITY);
        assert_eq!((cloud.len(), dropped), (1, 1));
    }

    #[test]
    fn the_procedural_ground_is_deterministic_dense_and_flat_where_figures_stand() {
        let ground = ProceduralGround {
            seed: 1,
            splats: 20_000,
            half_extent_m: 14.0,
        };
        let a = ground.load().unwrap();
        let b = ground.load().unwrap();
        assert_eq!(a.len(), 20_000);
        assert_eq!(a, b);
        assert!(procedural_height(0.0, 0.0).abs() < 0.03);
        assert!(procedural_height(2.0, -2.0).abs() < 0.03);
        let far_relief = (6..14)
            .map(|d| procedural_height(d as f32, 0.0).abs())
            .fold(0.0f32, f32::max);
        assert!(far_relief > 0.05, "uneven further out, got {far_relief}");
    }

    fn scattered(count: u32) -> Vec<Splat> {
        let mut random = Lcg(42);
        (0..count)
            .map(|_| Splat {
                centre: Vec3::new(
                    random.next() * 20.0 - 10.0,
                    random.next() * 4.0,
                    random.next() * 20.0 - 10.0,
                ),
                colour: [255; 4],
                covariance: [0.01, 0.0, 0.0, 0.01, 0.0, 0.01],
            })
            .collect()
    }

    #[test]
    fn the_counting_sort_matches_a_brute_force_sort_on_the_same_quantised_depth() {
        let splats = scattered(5_000);
        let eye = Vec3::new(0.0, 3.0, 14.0);
        let forward = Vec3::new(0.1, -0.2, -1.0).normalized();
        let mut sorter = SplatSorter::default();
        let order = sorter.sort(&splats, splats.len(), eye, forward).to_vec();
        assert_eq!(order, brute_force_order(&splats, eye, forward));
    }

    #[test]
    fn sorted_depth_never_increases_beyond_one_quantisation_step() {
        let splats = scattered(5_000);
        let eye = Vec3::new(3.0, 6.0, 20.0);
        let forward = Vec3::new(-0.1, -0.3, -1.0).normalized();
        let mut sorter = SplatSorter::default();
        let order = sorter.sort(&splats, splats.len(), eye, forward).to_vec();
        let depths: Vec<f32> = order
            .iter()
            .map(|i| (splats[*i as usize].centre - eye).dot(forward))
            .collect();
        let step = (depths[0] - depths[depths.len() - 1]) / 65535.0 + 1e-4;
        assert!(depths.windows(2).all(|pair| pair[1] <= pair[0] + step));
        assert!(depths[0] > depths[depths.len() - 1]);
    }

    #[test]
    fn splats_behind_the_camera_are_left_out_and_the_budget_limits_the_prefix() {
        let splats = vec![
            Splat {
                centre: Vec3::new(0.0, 0.0, -5.0),
                colour: [0; 4],
                covariance: [0.0; 6],
            },
            Splat {
                centre: Vec3::new(0.0, 0.0, 5.0),
                colour: [0; 4],
                covariance: [0.0; 6],
            },
            Splat {
                centre: Vec3::new(0.0, 0.0, -9.0),
                colour: [0; 4],
                covariance: [0.0; 6],
            },
        ];
        let mut sorter = SplatSorter::default();
        let eye = Vec3::ZERO;
        let forward = Vec3::new(0.0, 0.0, -1.0);
        assert_eq!(sorter.sort(&splats, 3, eye, forward), &[2, 0]);
        assert_eq!(sorter.sort(&splats, 1, eye, forward), &[0]);
        assert!(sorter.sort(&splats, 0, eye, forward).is_empty());
    }

    #[test]
    fn the_sorter_resorts_only_when_the_view_moved_enough() {
        let splats = scattered(100);
        let mut sorter = SplatSorter::default();
        let eye = Vec3::new(0.0, 3.0, 14.0);
        let forward = Vec3::new(0.0, -0.2, -1.0).normalized();
        assert!(sorter.needs_sort(eye, forward));
        sorter.sort(&splats, 100, eye, forward);
        assert!(!sorter.needs_sort(eye + Vec3::new(0.05, 0.0, 0.0), forward));
        assert!(sorter.needs_sort(eye + Vec3::new(0.5, 0.0, 0.0), forward));
        let turned = Vec3::new(0.1, -0.2, -1.0).normalized();
        assert!(sorter.needs_sort(eye, turned));
        sorter.invalidate();
        assert!(sorter.needs_sort(eye, forward));
    }

    fn half_to_f32(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exponent = i32::from((bits >> 10) & 0x1f);
        let mantissa = f32::from(bits & 0x3ff);
        match exponent {
            0 => sign * mantissa * 2f32.powi(-24),
            31 => sign * f32::INFINITY,
            _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
        }
    }

    #[test]
    fn half_precision_packing_round_trips_scales_and_saturates() {
        for value in [0.0f32, 1.0, 0.5, 0.0123, 3.7, 0.0001, -2.5, 1000.0] {
            let back = half_to_f32(f32_to_f16_bits(value));
            assert!(
                (back - value).abs() <= value.abs() * 0.001 + 1e-7,
                "{value} -> {back}"
            );
        }
        assert_eq!(half_to_f32(f32_to_f16_bits(1e9)), 65504.0);
        assert_eq!(f32_to_f16_bits(f32::NAN) & 0x7c00, 0x7c00);
    }

    #[test]
    fn packed_records_hold_position_colour_rotation_and_scale() {
        let rotation = Quat::new(0.1, -0.5, 0.3, 0.8).normalized();
        let words = pack_record(
            Vec3::new(1.5, -2.0, 3.25),
            Vec3::new(0.05, 0.2, 1.5),
            rotation,
            [10, 20, 30, 200],
        );
        assert_eq!(f32::from_bits(words[0]), 1.5);
        assert_eq!(f32::from_bits(words[2]), 3.25);
        assert_eq!(words[3].to_le_bytes(), [10, 20, 30, 200]);
        let snorm = |bits: u32| f32::from(bits as u16 as i16) / 32767.0;
        assert!((snorm(words[4]) - rotation.x).abs() < 1e-4);
        assert!((snorm(words[4] >> 16) - rotation.y).abs() < 1e-4);
        assert!((snorm(words[5] >> 16) - rotation.w).abs() < 1e-4);
        assert!((half_to_f32(words[6] as u16) - 0.05).abs() < 1e-4);
        assert!((half_to_f32((words[7] & 0xffff) as u16) - 1.5).abs() < 1e-3);
    }

    #[test]
    fn packing_applies_the_transform_and_counts_dropped_splats() {
        let bad = RawSplat {
            scale: Vec3::new(f32::NAN, 1.0, 1.0),
            ..raw(0.0, 0.0, 0.0)
        };
        let transform = SplatTransform {
            scale: 2.0,
            translate: Vec3::new(0.0, 0.0, 5.0),
            ..SplatTransform::IDENTITY
        };
        let (words, dropped) = pack_records(&[raw(1.0, 1.0, 0.0), bad], transform);
        assert_eq!((words.len(), dropped), (RECORD_WORDS, 1));
        assert_eq!(f32::from_bits(words[0]), 2.0);
        assert_eq!(f32::from_bits(words[2]), 5.0);
        assert!(
            (half_to_f32(words[6] as u16) - 0.2).abs() < 1e-3,
            "scale doubled"
        );
    }

    #[test]
    fn orientation_turns_a_y_down_scan_upright_and_the_transform_inverts() {
        // A scan with its up axis along -Y: rotating 180 degrees about X makes -Y point up.
        let orient = orientation_from_euler_deg([180.0, 0.0, 0.0]);
        let transform = SplatTransform {
            scale: 0.5,
            offset_y: 1.0,
            rot_y_deg: 30.0,
            translate: Vec3::new(2.0, 0.0, -3.0),
            orient,
        };
        let up = transform.rotation().rotate(Vec3::new(0.0, -1.0, 0.0));
        assert!((up.y - 1.0).abs() < 1e-5 && up.x.abs() < 1e-5, "{up:?}");
        let scan = Vec3::new(4.0, -6.0, 9.0);
        let back = transform.to_scan(transform.to_world(scan));
        assert!((back - scan).length() < 1e-4, "{back:?}");
        let (matrix, translation) = transform.affine();
        let world = transform.to_world(scan);
        let by_matrix = Vec3::new(
            matrix[0][0] * scan.x + matrix[0][1] * scan.y + matrix[0][2] * scan.z,
            matrix[1][0] * scan.x + matrix[1][1] * scan.y + matrix[1][2] * scan.z,
            matrix[2][0] * scan.x + matrix[2][1] * scan.y + matrix[2][2] * scan.z,
        ) + translation;
        assert!((by_matrix - world).length() < 1e-4);
    }

    #[test]
    fn the_gpu_depth_key_orders_back_to_front_with_millimetre_steps_near_the_camera() {
        let range = 900.0;
        let mut random = Lcg(11);
        let depths: Vec<f32> = (0..20_000).map(|_| 0.2 + random.next() * 300.0).collect();
        let keys: Vec<u32> = depths.iter().map(|d| depth_sort_key(*d, range)).collect();
        let order = counting_order(&keys);
        assert_eq!(order.len(), depths.len());
        // Back to front: depth never increases along the order by more than one key step.
        for pair in order.windows(2) {
            let (far, near) = (depths[pair[0] as usize], depths[pair[1] as usize]);
            let step = 2.0 * (far * range).sqrt() / 65535.0 + 1e-3;
            assert!(near <= far + step, "{far} then {near}");
        }
        // The nearest and farthest splats land at the two ends.
        assert_eq!(
            depths[order[0] as usize],
            depths.iter().cloned().fold(f32::MIN, f32::max)
        );
        let last = *order.last().unwrap() as usize;
        assert_eq!(
            depths[last],
            depths.iter().cloned().fold(f32::MAX, f32::min)
        );
        // Resolution: one key step is a few millimetres at 10 m and under 3 cm at 300 m.
        let step_at = |d: f32| (depth_sort_key(d, range) - depth_sort_key(d + 0.05, range)) as f32;
        assert!(step_at(10.0) >= 1.0 && depth_sort_key(0.0, range) == 65535);
        assert!(
            depth_sort_key(2000.0, range) == 0,
            "beyond the range everything shares the far key"
        );
    }

    #[test]
    fn counting_order_groups_equal_keys_and_keeps_them_in_input_order() {
        assert_eq!(counting_order(&[5, 1, 5, 0, 1]), vec![3, 1, 4, 0, 2]);
        assert!(counting_order(&[]).is_empty());
    }
}
