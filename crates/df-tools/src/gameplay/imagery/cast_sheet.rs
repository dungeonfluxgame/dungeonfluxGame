//! Deterministic photographic identity contact sheets. Labels are reference slots, never scene text.
use super::{ReferenceImage, party_sheet::SheetError};
use bytes::Bytes;
use df_providers::ImageFormat;
use image::{
    DynamicImage, ImageFormat as Container, Rgb, RgbImage, codecs::jpeg::JpegEncoder, imageops,
};
use sha2::{Digest, Sha256};
use std::io::Cursor;

/// One canonical cast member and the exact source photograph pinned for this request.
pub(in crate::gameplay) struct CastIdentity {
    pub(in crate::gameplay) id: String,
    pub(in crate::gameplay) figure: ReferenceImage,
}

const CELL_WIDTH: u32 = 512;
const CELL_HEIGHT: u32 = 768;
const LABEL_HEIGHT: u32 = 48;
const MAXIMUM_CAST: usize = 24;
const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];

/// Cells are sorted by canonical ID, labelled 1..N. The legend contains source digests so a
/// replacement photograph or identity mapping invalidates the request even for identical pixels.
pub(in crate::gameplay) fn cast_sheet(
    mut cast: Vec<CastIdentity>,
) -> Result<(ReferenceImage, String), SheetError> {
    if cast.is_empty() || cast.len() > MAXIMUM_CAST {
        return Err(SheetError::FigureCount);
    }
    cast.sort_by(|a, b| a.id.cmp(&b.id));
    if cast.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err(SheetError::FigureCount);
    }
    let columns = cast.len().min(4) as u32;
    let rows = (cast.len() as u32).div_ceil(columns);
    let mut sheet =
        RgbImage::from_pixel(columns * CELL_WIDTH, rows * CELL_HEIGHT, Rgb([36, 36, 36]));
    let mut legend = Vec::new();
    let mut sources = Sha256::new();
    for (index, identity) in cast.iter().enumerate() {
        let container = match identity.figure.format {
            ImageFormat::Png => Container::Png,
            ImageFormat::Jpeg => Container::Jpeg,
            ImageFormat::Webp => Container::WebP,
        };
        let source = image::load_from_memory_with_format(&identity.figure.bytes, container)
            .map_err(|_| SheetError::Decode)?;
        let picture = source
            .thumbnail(CELL_WIDTH, CELL_HEIGHT - LABEL_HEIGHT)
            .to_rgb8();
        let left = index as u32 % columns * CELL_WIDTH;
        let top = index as u32 / columns * CELL_HEIGHT;
        imageops::replace(
            &mut sheet,
            &picture,
            i64::from(left + (CELL_WIDTH - picture.width()) / 2),
            i64::from(top + LABEL_HEIGHT),
        );
        for (place, digit) in (index + 1).to_string().bytes().enumerate() {
            for (row, bits) in DIGITS[usize::from(digit - b'0')].iter().enumerate() {
                for column in 0..3 {
                    if bits & (1 << (2 - column)) != 0 {
                        for y in 0..6 {
                            for x in 0..6 {
                                sheet.put_pixel(
                                    left + 16 + place as u32 * 24 + column * 6 + x,
                                    top + 8 + row as u32 * 6 + y,
                                    Rgb([240, 240, 240]),
                                );
                            }
                        }
                    }
                }
            }
        }
        sources.update(identity.id.as_bytes());
        sources.update([0]);
        sources.update(identity.figure.sha256.as_bytes());
        sources.update([0]);
        legend.push(format!("slot {} = {}", index + 1, identity.id));
    }
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut Cursor::new(&mut bytes), 90)
        .encode_image(&DynamicImage::ImageRgb8(sheet))
        .map_err(|_| SheetError::Encode)?;
    if bytes.len() as u64 > super::references::MAXIMUM_REFERENCE_BYTES {
        return Err(SheetError::Encode);
    }
    Ok((
        ReferenceImage::new(ImageFormat::Jpeg, Bytes::from(bytes)),
        format!(
            "{}; identity revision {:x}",
            legend.join("; "),
            sources.finalize()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(id: &str, color: u8) -> CastIdentity {
        let picture = DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, Rgb([color, 20, 30])));
        let mut bytes = Cursor::new(Vec::new());
        picture.write_to(&mut bytes, Container::Png).unwrap();
        CastIdentity {
            id: id.to_owned(),
            figure: ReferenceImage::new(ImageFormat::Png, Bytes::from(bytes.into_inner())),
        }
    }
    #[test]
    fn complete_cast_is_stable_ordered_and_source_changes_invalidate_it() {
        let (sheet, legend) = cast_sheet(vec![
            identity("npc:courier", 10),
            identity("hero:2", 20),
            identity("npc:bandit", 30),
            identity("hero:1", 40),
        ])
        .unwrap();
        assert!(legend.starts_with("slot 1 = hero:1"));
        assert!(legend.contains("slot 4 = npc:courier"));
        let (reordered, same_legend) = cast_sheet(vec![
            identity("hero:1", 40),
            identity("hero:2", 20),
            identity("npc:bandit", 30),
            identity("npc:courier", 10),
        ])
        .unwrap();
        assert_eq!(sheet.sha256, reordered.sha256);
        assert_eq!(legend, same_legend);
        let (changed, _) = cast_sheet(vec![
            identity("hero:1", 99),
            identity("hero:2", 20),
            identity("npc:bandit", 30),
            identity("npc:courier", 10),
        ])
        .unwrap();
        assert_ne!(sheet.sha256, changed.sha256);
        assert_eq!(
            image::load_from_memory(&sheet.bytes).unwrap().width(),
            CELL_WIDTH * 4
        );
    }
    #[test]
    fn duplicate_or_missing_cast_cannot_be_silently_omitted() {
        assert!(cast_sheet(vec![]).is_err());
        assert!(cast_sheet(vec![identity("hero:1", 10), identity("hero:1", 20)]).is_err());
    }
    #[test]
    fn reviewed_harbor_photographs_supply_every_hero_and_npc_identity() {
        let directory =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated");
        let store = super::super::ReferenceStore::new(directory);
        let mut cast = Vec::new();
        for (id, source) in [
            ("hero:1", "figure-hero-1"),
            ("hero:2", "figure-hero-2"),
            ("hero:3", "figure-hero-3"),
            ("npc:courier", "sprite-courier"),
            ("npc:bandit", "sprite-bandit"),
        ] {
            cast.push(CastIdentity {
                id: id.to_owned(),
                figure: store.load(source).unwrap(),
            });
        }
        let (sheet, legend) = cast_sheet(cast).unwrap();
        assert!(legend.contains("slot 5 = npc:courier"));
        assert!(legend.contains("hero:2"));
        let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/plate-continuity");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("harbor-canonical-cast.jpg"), &sheet.bytes).unwrap();
        std::fs::write(output.join("harbor-canonical-cast.legend.txt"), legend).unwrap();
    }
}
