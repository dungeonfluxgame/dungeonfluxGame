//! The 3D Gaussian Splatting binary PLY loader, and the frame helper for World Labs Marble worlds.
//!
//! A Marble export (and any standard 3DGS trainer) writes a header, then one packed record of
//! little-endian `f32` per splat: position `x y z`, colour as the zeroth spherical harmonic
//! `f_dc_0..2`, opacity as a logit, scales as logs (`scale_0..2`) and the rotation quaternion as
//! `rot_0..3` in w, x, y, z order. Higher spherical harmonics (`f_rest_*`) and normals, when
//! present, are skipped over by position: columns are looked up by name, not by index.
//!
//! The file is untrusted input. The size, header length, property count, element count and the
//! exact body length are all checked before anything is allocated, and every failure is a typed
//! [`PlyError`]; nothing panics on bad bytes.
//!
//! Marble's frame is OpenCV (y down, z forward). [`marble_world_transform`] turns it into the
//! upright metre frame the battlefield uses: a half turn about X, the world's metric scale, and
//! the ground plane offset as the Y offset so the floor sits at y = 0.

use super::{
    math::{Quat, Vec3},
    splat::{MAX_SPLATS, RawSplat, SplatError, SplatLoader, SplatTransform},
};

/// The largest PLY accepted, the same ceiling as the `.splat` fetch. A file at the splat limit
/// (4 million splats of 56 bytes) is 224 MB.
pub const MAX_PLY_BYTES: usize = 256 * 1024 * 1024;
const MAXIMUM_HEADER_BYTES: usize = 16 * 1024;
const MAXIMUM_PROPERTIES: usize = 256;
const SPHERICAL_HARMONIC_C0: f32 = 0.282_094_8;
const END_HEADER: &[u8] = b"end_header";
const LARGEST_SCALE_FACTOR: f32 = 100.0;
const LARGEST_GROUND_OFFSET_M: f32 = 1_000.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlyError {
    FileTooLarge,
    /// The bytes do not start with a `ply` line.
    NotPly,
    /// No `end_header` line within the first 16 KiB.
    HeaderUnterminated,
    HeaderNotUtf8,
    /// A header line that is not `format`, `comment`, `obj_info`, `element` or `property`, or one
    /// of those with the wrong number of fields.
    BadHeaderLine,
    /// Anything but `binary_little_endian 1.0`.
    UnsupportedFormat,
    /// An element other than `vertex`.
    UnsupportedElement,
    DuplicateElement,
    MissingVertexElement,
    ZeroSplats,
    TooMany,
    TooManyProperties,
    /// A property that is not a plain float (a list, or another scalar type).
    UnsupportedProperty,
    DuplicateProperty,
    /// A property a splat needs is not in the header.
    MissingProperty(&'static str),
    /// The body is shorter than the header's count needs.
    Truncated,
    /// The body is longer than the header's count needs.
    TrailingBytes,
    NonFinite,
}

impl PlyError {
    fn reason(self) -> &'static str {
        match self {
            Self::FileTooLarge => "ply file too large",
            Self::NotPly => "not a ply file",
            Self::HeaderUnterminated => "ply header not terminated",
            Self::HeaderNotUtf8 => "ply header not utf-8",
            Self::BadHeaderLine => "ply header line",
            Self::UnsupportedFormat => "ply not binary_little_endian 1.0",
            Self::UnsupportedElement => "ply element is not vertex",
            Self::DuplicateElement => "ply vertex element repeated",
            Self::MissingVertexElement => "ply has no vertex element",
            Self::ZeroSplats => "ply has no splats",
            Self::TooMany => "ply splat count",
            Self::TooManyProperties => "ply property count",
            Self::UnsupportedProperty => "ply property is not a float",
            Self::DuplicateProperty => "ply property repeated",
            Self::MissingProperty(name) => name,
            Self::Truncated => "ply body too short",
            Self::TrailingBytes => "ply body too long",
            Self::NonFinite => "ply value not finite",
        }
    }
}

impl From<PlyError> for SplatError {
    fn from(error: PlyError) -> Self {
        match error {
            PlyError::FileTooLarge | PlyError::TooMany => Self::TooMany,
            PlyError::Truncated | PlyError::TrailingBytes => Self::BadLength,
            PlyError::NonFinite => Self::NonFinite,
            other => Self::MetaInvalid(other.reason()),
        }
    }
}

/// Where each field sits among the record's floats.
struct Columns {
    position: [usize; 3],
    colour: [usize; 3],
    opacity: usize,
    scale: [usize; 3],
    rotation: [usize; 4],
}

struct Header {
    count: usize,
    /// Floats per record.
    width: usize,
    columns: Columns,
    body_start: usize,
}

fn column(names: &[&str], wanted: &'static str) -> Result<usize, PlyError> {
    names
        .iter()
        .position(|name| *name == wanted)
        .ok_or(PlyError::MissingProperty(wanted))
}

fn parse_header(bytes: &[u8]) -> Result<Header, PlyError> {
    if bytes.len() > MAX_PLY_BYTES {
        return Err(PlyError::FileTooLarge);
    }
    if !bytes.starts_with(b"ply") {
        return Err(PlyError::NotPly);
    }
    let window = bytes.get(..MAXIMUM_HEADER_BYTES).unwrap_or(bytes);
    let marker = (0..window.len())
        .find(|&at| {
            window
                .get(at..)
                .is_some_and(|rest| rest.starts_with(END_HEADER))
                && (at == 0 || window.get(at - 1) == Some(&b'\n'))
        })
        .ok_or(PlyError::HeaderUnterminated)?;
    let after = marker + END_HEADER.len();
    let body_start = match (window.get(after), window.get(after + 1)) {
        (Some(b'\n'), _) => after + 1,
        (Some(b'\r'), Some(b'\n')) => after + 2,
        _ => return Err(PlyError::HeaderUnterminated),
    };
    let text = window
        .get(..marker)
        .and_then(|header| std::str::from_utf8(header).ok())
        .ok_or(PlyError::HeaderNotUtf8)?;

    let mut format_seen = false;
    let mut count: Option<usize> = None;
    let mut in_vertex = false;
    let mut names: Vec<&str> = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if number == 0 {
            if fields != ["ply"] {
                return Err(PlyError::NotPly);
            }
            continue;
        }
        match fields.as_slice() {
            ["comment", ..] | ["obj_info", ..] => {}
            ["format", "binary_little_endian", "1.0"] => format_seen = true,
            ["format", ..] => return Err(PlyError::UnsupportedFormat),
            ["element", name, size] => {
                if !format_seen {
                    return Err(PlyError::UnsupportedFormat);
                }
                if *name != "vertex" {
                    return Err(PlyError::UnsupportedElement);
                }
                if count.is_some() {
                    return Err(PlyError::DuplicateElement);
                }
                count = Some(size.parse().map_err(|_| PlyError::BadHeaderLine)?);
                in_vertex = true;
            }
            ["property", "list", ..] => return Err(PlyError::UnsupportedProperty),
            ["property", kind, name] => {
                if !in_vertex {
                    return Err(PlyError::BadHeaderLine);
                }
                if !matches!(*kind, "float" | "float32") {
                    return Err(PlyError::UnsupportedProperty);
                }
                if names.contains(name) {
                    return Err(PlyError::DuplicateProperty);
                }
                if names.len() >= MAXIMUM_PROPERTIES {
                    return Err(PlyError::TooManyProperties);
                }
                names.push(name);
            }
            _ => return Err(PlyError::BadHeaderLine),
        }
    }
    if !format_seen {
        return Err(PlyError::UnsupportedFormat);
    }
    let count = count.ok_or(PlyError::MissingVertexElement)?;
    if count == 0 {
        return Err(PlyError::ZeroSplats);
    }
    if count > MAX_SPLATS {
        return Err(PlyError::TooMany);
    }
    let by_name = |wanted| column(&names, wanted);
    let columns = Columns {
        position: [by_name("x")?, by_name("y")?, by_name("z")?],
        colour: [by_name("f_dc_0")?, by_name("f_dc_1")?, by_name("f_dc_2")?],
        opacity: by_name("opacity")?,
        scale: [
            by_name("scale_0")?,
            by_name("scale_1")?,
            by_name("scale_2")?,
        ],
        rotation: [
            by_name("rot_0")?,
            by_name("rot_1")?,
            by_name("rot_2")?,
            by_name("rot_3")?,
        ],
    };
    Ok(Header {
        count,
        width: names.len(),
        columns,
        body_start,
    })
}

/// Loads the standard 3DGS binary PLY. See the module notes for the layout.
pub struct PlyLoader<'a> {
    pub bytes: &'a [u8],
}

impl PlyLoader<'_> {
    /// Like [`SplatLoader::load`] with the precise failure.
    pub fn parse(&self) -> Result<Vec<RawSplat>, PlyError> {
        let header = parse_header(self.bytes)?;
        let body = self
            .bytes
            .get(header.body_start..)
            .ok_or(PlyError::Truncated)?;
        // `width` is at most MAXIMUM_PROPERTIES and `count` at most MAX_SPLATS: no overflow.
        let stride = header.width * 4;
        let expected = header.count * stride;
        if body.len() < expected {
            return Err(PlyError::Truncated);
        }
        if body.len() > expected {
            return Err(PlyError::TrailingBytes);
        }
        let columns = &header.columns;
        let mut splats = Vec::with_capacity(header.count);
        for record in body.chunks_exact(stride) {
            let (floats, _) = record.as_chunks::<4>();
            // Every column index is below `width`, which is the number of floats in a record.
            let value = |at: usize| f32::from_le_bytes(floats[at]);
            let triple = |at: [usize; 3]| Vec3::new(value(at[0]), value(at[1]), value(at[2]));
            let position = triple(columns.position);
            let logit = value(columns.opacity);
            let dc = triple(columns.colour);
            let log_scale = triple(columns.scale);
            let [w, x, y, z] = columns.rotation.map(value);
            if !(position.is_finite()
                && dc.is_finite()
                && log_scale.is_finite()
                && logit.is_finite()
                && [w, x, y, z].into_iter().all(f32::is_finite))
            {
                return Err(PlyError::NonFinite);
            }
            let scale = Vec3::new(log_scale.x.exp(), log_scale.y.exp(), log_scale.z.exp());
            if !scale.is_finite() {
                return Err(PlyError::NonFinite);
            }
            let colour = |dc: f32| {
                ((0.5 + SPHERICAL_HARMONIC_C0 * dc).clamp(0.0, 1.0) * 255.0).round() as u8
            };
            let opacity = 1.0 / (1.0 + (-logit).exp());
            splats.push(RawSplat {
                position,
                scale,
                rotation: Quat::new(x, y, z, w).normalized(),
                colour: [
                    colour(dc.x),
                    colour(dc.y),
                    colour(dc.z),
                    (opacity * 255.0).round() as u8,
                ],
            });
        }
        Ok(splats)
    }
}

impl SplatLoader for PlyLoader<'_> {
    fn load(&self) -> Result<Vec<RawSplat>, SplatError> {
        self.parse().map_err(SplatError::from)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorldFrameError {
    /// The metric scale factor is not a finite number in (0, 100].
    ScaleFactor,
    /// The ground plane offset is not a finite number within 1,000 m.
    GroundOffset,
}

/// The transform that stands a Marble world upright in metres, from the two numbers in the
/// world's `semantics_metadata`.
///
/// A scan point is turned half a turn about X (OpenCV's y down, z forward becomes y up, z back),
/// multiplied by `metric_scale_factor`, then raised by `ground_plane_offset`, so the floor of
/// the room lies at y = 0. The same transform applies to the PLY and to the collider GLB, which
/// share the frame. In a scene file this is `scale`, `offset_y` and `orient_deg: [180, 0, 0]`.
pub fn marble_world_transform(
    metric_scale_factor: f32,
    ground_plane_offset: f32,
) -> Result<SplatTransform, WorldFrameError> {
    if !metric_scale_factor.is_finite()
        || metric_scale_factor <= 0.0
        || metric_scale_factor > LARGEST_SCALE_FACTOR
    {
        return Err(WorldFrameError::ScaleFactor);
    }
    if !ground_plane_offset.is_finite() || ground_plane_offset.abs() > LARGEST_GROUND_OFFSET_M {
        return Err(WorldFrameError::GroundOffset);
    }
    Ok(SplatTransform {
        scale: metric_scale_factor,
        offset_y: ground_plane_offset,
        rot_y_deg: 0.0,
        translate: Vec3::ZERO,
        // Exactly (x, y, z) -> (x, -y, -z); a trigonometric 180 degrees leaves rounding dust.
        orient: Quat::new(1.0, 0.0, 0.0, 0.0),
    })
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_1_SQRT_2;

    use super::*;

    const PROPERTIES: [&str; 14] = [
        "x", "y", "z", "f_dc_0", "f_dc_1", "f_dc_2", "opacity", "scale_0", "scale_1", "scale_2",
        "rot_0", "rot_1", "rot_2", "rot_3",
    ];

    fn header(names: &[&str], count: usize) -> String {
        let mut text = format!("ply\nformat binary_little_endian 1.0\nelement vertex {count}\n");
        for name in names {
            text.push_str(&format!("property float {name}\n"));
        }
        text.push_str("end_header\n");
        text
    }

    /// One record in the Marble layout: position, f_dc, opacity logit, log scale, w x y z.
    fn record(position: [f32; 3], dc: f32, logit: f32, log_scale: f32, wxyz: [f32; 4]) -> Vec<f32> {
        let mut floats = position.to_vec();
        floats.extend([dc; 3]);
        floats.push(logit);
        floats.extend([log_scale; 3]);
        floats.extend(wxyz);
        floats
    }

    fn file(names: &[&str], records: &[Vec<f32>]) -> Vec<u8> {
        let mut bytes = header(names, records.len()).into_bytes();
        for float in records.iter().flatten() {
            bytes.extend_from_slice(&float.to_le_bytes());
        }
        bytes
    }

    fn marble(records: &[Vec<f32>]) -> Vec<u8> {
        file(&PROPERTIES, records)
    }

    fn one() -> Vec<f32> {
        record([1.0, -2.0, 3.0], 0.0, 0.0, 0.0, [1.0, 0.0, 0.0, 0.0])
    }

    fn parse(bytes: &[u8]) -> Result<Vec<RawSplat>, PlyError> {
        PlyLoader { bytes }.parse()
    }

    #[test]
    fn a_synthetic_marble_file_decodes_every_field() {
        let bytes = marble(&[
            one(),
            // Opaque, black, small, turned 90 degrees about Y (w = cos 45, y = sin 45).
            record(
                [0.5, 0.25, -8.0],
                -10.0,
                30.0,
                -3.0,
                [FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2, 0.0],
            ),
        ]);
        assert_eq!(bytes.len(), header(&PROPERTIES, 2).len() + 2 * 56);
        let splats = parse(&bytes).unwrap();
        assert_eq!(splats.len(), 2);

        let first = splats[0];
        assert_eq!(first.position, Vec3::new(1.0, -2.0, 3.0));
        assert_eq!(first.scale, Vec3::new(1.0, 1.0, 1.0));
        // SH 0 is mid grey, a zero logit is half opacity.
        assert_eq!(first.colour, [128, 128, 128, 128]);
        assert_eq!(first.rotation, Quat::IDENTITY);

        let second = splats[1];
        assert_eq!(second.position, Vec3::new(0.5, 0.25, -8.0));
        assert!((second.scale.x - (-3.0f32).exp()).abs() < 1e-6);
        assert_eq!(second.colour, [0, 0, 0, 255]);
        // rot_0..3 is w, x, y, z.
        assert!((second.rotation.w - FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((second.rotation.y - FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(second.rotation.x, 0.0);
    }

    #[test]
    fn the_trait_path_gives_the_same_splats() {
        let bytes = marble(&[one()]);
        let via_trait = PlyLoader { bytes: &bytes }.load().unwrap();
        assert_eq!(via_trait, parse(&bytes).unwrap());
    }

    #[test]
    fn extra_and_reordered_columns_are_found_by_name() {
        let names = [
            "nx", "rot_3", "opacity", "x", "y", "z", "f_rest_0", "f_dc_0", "f_dc_1", "f_dc_2",
            "scale_0", "scale_1", "scale_2", "rot_0", "rot_1", "rot_2",
        ];
        let mut floats = vec![0.0f32; names.len()];
        let mut set = |name: &str, value: f32| {
            let at = names.iter().position(|n| *n == name).unwrap();
            floats[at] = value;
        };
        set("nx", 9.0);
        set("f_rest_0", 9.0);
        set("x", 4.0);
        set("y", 5.0);
        set("z", 6.0);
        set("rot_0", 1.0);
        let splats = parse(&file(&names, &[floats])).unwrap();
        assert_eq!(splats[0].position, Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(splats[0].rotation, Quat::IDENTITY);
        assert_eq!(splats[0].colour, [128, 128, 128, 128]);
    }

    #[test]
    fn a_crlf_header_and_comments_are_accepted() {
        let mut bytes =
            b"ply\r\nformat binary_little_endian 1.0\r\ncomment made by a test\r\nelement vertex 1\r\n"
                .to_vec();
        for name in PROPERTIES {
            bytes.extend_from_slice(format!("property float32 {name}\r\n").as_bytes());
        }
        bytes.extend_from_slice(b"end_header\r\n");
        for float in one() {
            bytes.extend_from_slice(&float.to_le_bytes());
        }
        assert_eq!(parse(&bytes).unwrap().len(), 1);
    }

    #[test]
    fn a_zero_rotation_falls_back_to_identity_not_nan() {
        let bytes = marble(&[record([0.0; 3], 0.0, 0.0, 0.0, [0.0; 4])]);
        assert_eq!(parse(&bytes).unwrap()[0].rotation, Quat::IDENTITY);
    }

    #[test]
    fn bytes_that_are_not_a_ply_are_refused() {
        assert_eq!(parse(&[]), Err(PlyError::NotPly));
        assert_eq!(parse(b"glTF\x02\x00\x00\x00"), Err(PlyError::NotPly));
        assert_eq!(parse(b"plyx\nformat"), Err(PlyError::HeaderUnterminated));
        assert_eq!(
            parse(b"ply\nformat binary_little_endian 1.0\n"),
            Err(PlyError::HeaderUnterminated)
        );
        assert_eq!(
            parse(b"plyx\nend_header\n"),
            Err(PlyError::NotPly),
            "the first line must be exactly ply"
        );
    }

    #[test]
    fn an_unsupported_format_element_or_property_is_refused() {
        let swap = |from: &str, to: &str| {
            let text = header(&PROPERTIES, 1).replace(from, to);
            let mut bytes = text.into_bytes();
            bytes.extend(std::iter::repeat_n(0u8, 56));
            bytes
        };
        assert_eq!(
            parse(&swap("binary_little_endian", "ascii")),
            Err(PlyError::UnsupportedFormat)
        );
        assert_eq!(
            parse(&swap("binary_little_endian", "binary_big_endian")),
            Err(PlyError::UnsupportedFormat)
        );
        assert_eq!(
            parse(&swap("element vertex", "element face")),
            Err(PlyError::UnsupportedElement)
        );
        assert_eq!(
            parse(&swap("property float opacity", "property double opacity")),
            Err(PlyError::UnsupportedProperty)
        );
        assert_eq!(
            parse(&swap(
                "property float opacity",
                "property list uchar int opacity"
            )),
            Err(PlyError::UnsupportedProperty)
        );
        assert_eq!(
            parse(&swap("property float opacity", "property float scale_0")),
            Err(PlyError::DuplicateProperty)
        );
        assert_eq!(
            parse(&swap("property float opacity", "garbage line")),
            Err(PlyError::BadHeaderLine)
        );
        assert_eq!(
            parse(&swap("element vertex 1", "element vertex lots")),
            Err(PlyError::BadHeaderLine)
        );
        assert_eq!(
            parse(&swap(
                "element vertex 1\n",
                "element vertex 1\nelement vertex 1\n"
            )),
            Err(PlyError::DuplicateElement)
        );
    }

    #[test]
    fn a_header_with_no_vertex_element_or_a_missing_property_is_refused() {
        let no_vertex = b"ply\nformat binary_little_endian 1.0\nend_header\n";
        assert_eq!(parse(no_vertex), Err(PlyError::MissingVertexElement));
        let no_format = b"ply\nend_header\n";
        assert_eq!(parse(no_format), Err(PlyError::UnsupportedFormat));
        let names: Vec<&str> = PROPERTIES
            .iter()
            .copied()
            .filter(|n| *n != "scale_1")
            .collect();
        let bytes = file(&names, &[vec![0.0; 13]]);
        assert_eq!(parse(&bytes), Err(PlyError::MissingProperty("scale_1")));
    }

    #[test]
    fn counts_are_bounded_before_anything_is_allocated() {
        let with_count = |count: usize| {
            let mut bytes = header(&PROPERTIES, count).into_bytes();
            bytes.extend(std::iter::repeat_n(0u8, 56));
            bytes
        };
        assert_eq!(parse(&with_count(0)), Err(PlyError::ZeroSplats));
        assert_eq!(parse(&with_count(MAX_SPLATS + 1)), Err(PlyError::TooMany));
        assert_eq!(parse(&with_count(usize::MAX)), Err(PlyError::TooMany));
        // A count that fits the limit but not the body is a short body, found without allocating.
        assert_eq!(parse(&with_count(MAX_SPLATS)), Err(PlyError::Truncated));
        let many: Vec<String> = (0..=MAXIMUM_PROPERTIES).map(|n| format!("p{n}")).collect();
        let names: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(parse(&file(&names, &[])), Err(PlyError::TooManyProperties));
    }

    #[test]
    fn a_short_or_long_body_is_refused() {
        let bytes = marble(&[one(), one()]);
        assert_eq!(parse(&bytes[..bytes.len() - 1]), Err(PlyError::Truncated));
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(parse(&long), Err(PlyError::TrailingBytes));
        assert_eq!(parse(&bytes).unwrap().len(), 2);
    }

    #[test]
    fn non_finite_values_are_refused() {
        for (at, value) in [
            (0, f32::NAN),
            (4, f32::INFINITY),
            (6, f32::NAN),
            (11, f32::NAN),
        ] {
            let mut floats = one();
            floats[at] = value;
            assert_eq!(
                parse(&marble(&[floats])),
                Err(PlyError::NonFinite),
                "float {at}"
            );
        }
        // A log scale of 100 is finite but exp() overflows f32.
        let overflow = record([0.0; 3], 0.0, 0.0, 100.0, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(parse(&marble(&[overflow])), Err(PlyError::NonFinite));
    }

    #[test]
    fn a_header_that_never_ends_within_the_limit_is_refused() {
        let mut bytes = b"ply\nformat binary_little_endian 1.0\n".to_vec();
        while bytes.len() < MAXIMUM_HEADER_BYTES + 100 {
            bytes.extend_from_slice(b"comment padding padding padding padding\n");
        }
        bytes.extend_from_slice(b"end_header\n");
        assert_eq!(parse(&bytes), Err(PlyError::HeaderUnterminated));
    }

    #[test]
    fn a_header_that_is_not_utf8_is_refused() {
        let mut bytes = b"ply\nformat binary_little_endian 1.0\ncomment ".to_vec();
        bytes.extend_from_slice(&[0xff, 0xfe]);
        bytes.extend_from_slice(b"\nend_header\n");
        assert_eq!(parse(&bytes), Err(PlyError::HeaderNotUtf8));
    }

    #[test]
    fn an_oversize_file_is_refused_by_length_alone() {
        let mut bytes = vec![0u8; MAX_PLY_BYTES + 1];
        bytes[..3].copy_from_slice(b"ply");
        assert_eq!(parse(&bytes), Err(PlyError::FileTooLarge));
    }

    #[test]
    fn ply_errors_map_to_the_loader_trait_errors() {
        assert_eq!(SplatError::from(PlyError::TooMany), SplatError::TooMany);
        assert_eq!(
            SplatError::from(PlyError::FileTooLarge),
            SplatError::TooMany
        );
        assert_eq!(SplatError::from(PlyError::Truncated), SplatError::BadLength);
        assert_eq!(SplatError::from(PlyError::NonFinite), SplatError::NonFinite);
        assert_eq!(
            SplatError::from(PlyError::MissingProperty("opacity")),
            SplatError::MetaInvalid("opacity")
        );
        assert_eq!(
            PlyLoader { bytes: b"nope" }.load().unwrap_err(),
            SplatError::MetaInvalid("not a ply file")
        );
    }

    #[test]
    fn the_marble_transform_stands_the_world_upright_on_the_floor() {
        let transform = marble_world_transform(1.5, 1.2).unwrap();
        // The camera is at the raw origin: 1.2 m above the floor after scaling and offsetting.
        assert_eq!(transform.to_world(Vec3::ZERO), Vec3::new(0.0, 1.2, 0.0));
        // The floor is below the camera, which is +y in OpenCV: raw y = offset / scale.
        let floor = transform.to_world(Vec3::new(2.0, 1.2 / 1.5, 4.0));
        assert!(floor.y.abs() < 1e-6);
        // x is kept, z flips, and everything scales.
        assert_eq!(floor.x, 3.0);
        assert_eq!(floor.z, -6.0);
        // Raw -y (up in the room) is up in the world.
        assert!(transform.to_world(Vec3::new(0.0, -2.0, 0.0)).y > 1.2);
        // The inverse maps back.
        let back = transform.to_scan(floor);
        assert!((back - Vec3::new(2.0, 0.8, 4.0)).length() < 1e-5);
    }

    #[test]
    fn the_marble_transform_matches_the_scene_file_orientation() {
        let transform = marble_world_transform(1.501_432_2, 1.208_879_7).unwrap();
        let from_scene_file = super::super::splat::orientation_from_euler_deg([180.0, 0.0, 0.0]);
        let point = Vec3::new(0.3, -0.7, 2.1);
        let a = transform.to_world(point);
        let b = SplatTransform {
            orient: from_scene_file,
            ..transform
        }
        .to_world(point);
        assert!((a - b).length() < 1e-5);
    }

    #[test]
    fn the_marble_transform_refuses_unusable_numbers() {
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 100.1] {
            assert_eq!(
                marble_world_transform(scale, 1.0),
                Err(WorldFrameError::ScaleFactor)
            );
        }
        for offset in [f32::NAN, f32::INFINITY, 1_000.5, -1_000.5] {
            assert_eq!(
                marble_world_transform(1.0, offset),
                Err(WorldFrameError::GroundOffset)
            );
        }
        assert!(marble_world_transform(1.0, -3.0).is_ok());
    }
}
