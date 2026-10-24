//! Deterministic photographic identity sheets: separate photographs in slot order, never scene text.
//!
//! Each photograph keeps its own picture, composited onto neutral photographic grey where its
//! source is transparent. No numbers or labels are drawn on the sheet: a numbered grid is the
//! collage look the plate model copied, so slot order is positional only (left to right, then
//! top to bottom) and the legend names that order in text outside the image.
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
const MAXIMUM_CAST: usize = 24;
/// Neutral photographic grey behind a transparent source, never black: black reads as a cut-out.
const NEUTRAL_GREY: u8 = 128;
/// Photographs are sorted by canonical ID; slot N is the Nth photograph in reading order. The legend contains source digests so a
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
    let columns = if cast.len() == 6 {
        3
    } else {
        cast.len().min(4)
    } as u32;
    let rows = (cast.len() as u32).div_ceil(columns);
    let mut sheet = RgbImage::from_pixel(
        columns * CELL_WIDTH,
        rows * CELL_HEIGHT,
        Rgb([NEUTRAL_GREY; 3]),
    );
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
        // Composite onto neutral grey, never black: to_rgb8 alone drops alpha onto black and
        // the photograph reads as a cut-out pasted on a black panel.
        let rgba = source.thumbnail(CELL_WIDTH, CELL_HEIGHT).to_rgba8();
        let mut picture = RgbImage::from_pixel(rgba.width(), rgba.height(), Rgb([NEUTRAL_GREY; 3]));
        for (target, pixel) in picture.pixels_mut().zip(rgba.pixels()) {
            let alpha = u16::from(pixel.0[3]);
            for channel in 0..3 {
                let blended = (u16::from(pixel.0[channel]) * alpha
                    + u16::from(NEUTRAL_GREY) * (255 - alpha))
                    / 255;
                target.0[channel] = u8::try_from(blended).unwrap_or(u8::MAX);
            }
        }
        let left = index as u32 % columns * CELL_WIDTH;
        let top = index as u32 / columns * CELL_HEIGHT;
        imageops::replace(
            &mut sheet,
            &picture,
            i64::from(left + (CELL_WIDTH - picture.width()) / 2),
            i64::from(top + (CELL_HEIGHT - picture.height()) / 2),
        );
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
    fn six_canonical_figures_share_one_sheet_in_two_rows_of_three() {
        let cast = (1..=6)
            .rev()
            .map(|seat| identity(&format!("hero:{seat}"), seat * 30))
            .collect();
        let (sheet, legend) = cast_sheet(cast).unwrap();
        let decoded = image::load_from_memory(&sheet.bytes).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (3 * CELL_WIDTH, 2 * CELL_HEIGHT)
        );
        for seat in 1..=6 {
            assert!(legend.contains(&format!("slot {seat} = hero:{seat}")));
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
    fn a_transparent_source_reads_as_a_photograph_on_neutral_grey_with_no_drawn_number() {
        // A transparent canonical figure must not land on black (a cut-out panel), and no
        // digit is drawn on the sheet for the plate model to copy.
        let picture = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([200, 30, 30, 0]),
        ));
        let mut bytes = Cursor::new(Vec::new());
        picture.write_to(&mut bytes, Container::Png).unwrap();
        let (sheet, legend) = cast_sheet(vec![CastIdentity {
            id: "hero:1".to_owned(),
            figure: ReferenceImage::new(ImageFormat::Png, Bytes::from(bytes.into_inner())),
        }])
        .unwrap();
        assert!(legend.starts_with("slot 1 = hero:1"));
        let decoded = image::load_from_memory(&sheet.bytes).unwrap().to_rgb8();
        for at in [(4, 4), (256, 24), (500, 700)] {
            let pixel = decoded.get_pixel(at.0, at.1).0;
            assert!(
                pixel
                    .iter()
                    .all(|channel| channel.abs_diff(NEUTRAL_GREY) < 12),
                "transparent source on neutral grey, no drawn number at {at:?}: {pixel:?}"
            );
        }
    }
    #[test]
    fn duplicate_or_missing_cast_cannot_be_silently_omitted() {
        assert!(cast_sheet(vec![]).is_err());
        assert!(cast_sheet(vec![identity("hero:1", 10), identity("hero:1", 20)]).is_err());
    }
    #[test]
    fn reviewed_harbor_photographs_supply_npc_identities_and_refuse_stock_heroes() {
        let directory =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated");
        let store = super::super::ReferenceStore::new(directory);
        // REF-058: player heroes come from their own character sheets, never stock figures.
        for stock in ["figure-hero-1", "figure-hero-2", "figure-hero-3"] {
            assert!(
                store.load(stock).is_err(),
                "{stock} must not be a hero reference"
            );
        }
        let mut cast = Vec::new();
        for (id, source) in [
            ("npc:courier", "sprite-courier"),
            ("npc:bandit", "sprite-bandit"),
        ] {
            cast.push(CastIdentity {
                id: id.to_owned(),
                figure: store.load(source).unwrap(),
            });
        }
        let (sheet, legend) = cast_sheet(cast).unwrap();
        assert!(legend.contains("npc:courier"));
        assert!(legend.contains("npc:bandit"));
        let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/plate-continuity");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("harbor-canonical-cast.jpg"), &sheet.bytes).unwrap();
        std::fs::write(output.join("harbor-canonical-cast.legend.txt"), legend).unwrap();
    }
}
