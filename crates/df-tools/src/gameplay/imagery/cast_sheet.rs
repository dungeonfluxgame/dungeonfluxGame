//! Ordered canonical photographs; gameplay keeps identity order and the separate text legend.

use df_imagery::{
    AlphaHandling, BlendMode, CanvasSpec, CompositeLayer, Dimensions, EncodedAlphaBlend, Encoding,
    ImageTransform, Purpose, PurposedUse, TransformSource, TransformSpec, VariantRole,
};
use sha2::{Digest, Sha256};

use super::{
    Imagery, ReferenceImage,
    party_sheet::{self, SheetError},
};

pub(in crate::gameplay) struct CastIdentity {
    pub(in crate::gameplay) id: String,
    pub(in crate::gameplay) figure: ReferenceImage,
}

const CELL_WIDTH: u32 = 512;
const CELL_HEIGHT: u32 = 768;
const MAXIMUM_CAST: usize = 24;

/// Canonical ID order defines photograph positions; source hashes remain in the separate legend.
pub(in crate::gameplay) async fn cast_sheet(
    imagery: &Imagery,
    mut cast: Vec<CastIdentity>,
) -> Result<(ReferenceImage, String), SheetError> {
    if cast.is_empty() || cast.len() > MAXIMUM_CAST {
        return Err(SheetError::FigureCount);
    }
    cast.sort_by(|left, right| left.id.cmp(&right.id));
    if cast.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err(SheetError::FigureCount);
    }
    let columns = u32::try_from(if cast.len() == 6 {
        3
    } else {
        cast.len().min(4)
    })
    .map_err(|_| SheetError::FigureCount)?;
    let rows = u32::try_from(cast.len())
        .map_err(|_| SheetError::FigureCount)?
        .div_ceil(columns);
    let mut legend = Vec::with_capacity(cast.len());
    let mut sources = Sha256::new();
    let mut layers = Vec::with_capacity(cast.len());
    for (index, identity) in cast.iter().enumerate() {
        let header = party_sheet::picture_header(&identity.figure)?;
        let ratio = (f64::from(CELL_WIDTH) / f64::from(header.width_px))
            .min(f64::from(CELL_HEIGHT) / f64::from(header.height_px));
        // The fit ratio bounds both rounded dimensions by the u32-sized cell.
        let width_px = (f64::from(header.width_px) * ratio).round().max(1.0) as u32;
        let height_px = (f64::from(header.height_px) * ratio).round().max(1.0) as u32;
        let mut steps = vec![ImageTransform::EncodedThumbnail(
            Dimensions::new(CELL_WIDTH, CELL_HEIGHT).map_err(|_| SheetError::Encode)?,
        )];
        if !header.has_alpha {
            // The original RGB thumbnail never introduced an alpha channel or its rounding.
            steps.push(ImageTransform::DiscardAlpha);
        }
        steps.push(ImageTransform::EncodedMatte {
            fill: party_sheet::grey(),
            blend: EncodedAlphaBlend::IntegerFloor,
        });
        let image = party_sheet::transform_reference(
            imagery,
            &identity.figure,
            steps,
            party_sheet::output(width_px, height_px, AlphaHandling::Opaque, Encoding::Png)?,
            "reference-prep:cast-photograph",
        )
        .await?;
        let source = imagery
            .pin_reference(&image)
            .await
            .map_err(SheetError::Execution)?;
        let slot = u32::try_from(index).map_err(|_| SheetError::FigureCount)?;
        let left_px = slot % columns * CELL_WIDTH + (CELL_WIDTH - width_px) / 2;
        let top_px = slot / columns * CELL_HEIGHT + (CELL_HEIGHT - height_px) / 2;
        layers.push(
            CompositeLayer::new(
                source,
                i32::try_from(left_px).map_err(|_| SheetError::Encode)?,
                i32::try_from(top_px).map_err(|_| SheetError::Encode)?,
                1000,
                BlendMode::SourceOver,
            )
            .map_err(|_| SheetError::Encode)?,
        );
        sources.update(identity.id.as_bytes());
        sources.update([0]);
        sources.update(identity.figure.sha256.as_bytes());
        sources.update([0]);
        legend.push(format!("slot {} = {}", index + 1, identity.id));
    }
    let (width_px, height_px) = (columns * CELL_WIDTH, rows * CELL_HEIGHT);
    let image = imagery
        .transform(
            "reference-prep:cast-sheet",
            PurposedUse {
                purpose: Purpose::ReferencePrep,
                role: VariantRole::Reference,
            },
            TransformSpec {
                source: TransformSource::Canvas(
                    CanvasSpec::new(width_px, height_px, party_sheet::grey())
                        .map_err(|_| SheetError::Encode)?,
                ),
                steps: layers
                    .chunks(df_imagery::DEFAULT_MAX_COMPOSITE_LAYERS)
                    .map(|layers| ImageTransform::Composite(layers.to_vec()))
                    .collect(),
            },
            party_sheet::output(
                width_px,
                height_px,
                AlphaHandling::Opaque,
                party_sheet::jpeg(90)?,
            )?,
        )
        .await
        .map_err(SheetError::Execution)?;
    if image.bytes.len() as u64 > super::references::MAXIMUM_REFERENCE_BYTES {
        return Err(SheetError::Encode);
    }
    Ok((
        image,
        format!(
            "{}; identity revision {:x}",
            legend.join("; "),
            sources.finalize()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::TransformFixture;
    use super::*;
    use bytes::Bytes;
    use df_providers::ImageFormat;
    use image::{DynamicImage, ImageFormat as Container, Rgb, RgbImage};
    use std::io::Cursor;
    const NEUTRAL_GREY: u8 = 128;
    async fn identity(fixture: &TransformFixture, id: &str, color: u8) -> CastIdentity {
        let picture = DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, Rgb([color, 20, 30])));
        let mut bytes = Cursor::new(Vec::new());
        picture.write_to(&mut bytes, Container::Png).unwrap();
        CastIdentity {
            id: id.to_owned(),
            figure: fixture
                .reference(ImageFormat::Png, Bytes::from(bytes.into_inner()))
                .await,
        }
    }
    #[tokio::test]
    async fn six_canonical_figures_share_one_sheet_in_two_rows_of_three() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
        let mut cast = Vec::new();
        for seat in (1..=6).rev() {
            cast.push(identity(&fixture, &format!("hero:{seat}"), seat * 30).await);
        }
        let (sheet, legend) = cast_sheet(imagery, cast).await.unwrap();
        let decoded = image::load_from_memory(&sheet.bytes).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (3 * CELL_WIDTH, 2 * CELL_HEIGHT)
        );
        for seat in 1..=6 {
            assert!(legend.contains(&format!("slot {seat} = hero:{seat}")));
        }
        fixture.close().await;
    }

    #[tokio::test]
    async fn complete_cast_is_stable_ordered_and_source_changes_invalidate_it() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
        let (sheet, legend) = cast_sheet(
            imagery,
            vec![
                identity(&fixture, "npc:courier", 10).await,
                identity(&fixture, "hero:2", 20).await,
                identity(&fixture, "npc:bandit", 30).await,
                identity(&fixture, "hero:1", 40).await,
            ],
        )
        .await
        .unwrap();
        assert!(legend.starts_with("slot 1 = hero:1"));
        assert!(legend.contains("slot 4 = npc:courier"));
        let (reordered, same_legend) = cast_sheet(
            imagery,
            vec![
                identity(&fixture, "hero:1", 40).await,
                identity(&fixture, "hero:2", 20).await,
                identity(&fixture, "npc:bandit", 30).await,
                identity(&fixture, "npc:courier", 10).await,
            ],
        )
        .await
        .unwrap();
        assert_eq!(sheet.sha256, reordered.sha256);
        assert_eq!(legend, same_legend);
        let (changed, _) = cast_sheet(
            imagery,
            vec![
                identity(&fixture, "hero:1", 99).await,
                identity(&fixture, "hero:2", 20).await,
                identity(&fixture, "npc:bandit", 30).await,
                identity(&fixture, "npc:courier", 10).await,
            ],
        )
        .await
        .unwrap();
        assert_ne!(sheet.sha256, changed.sha256);
        assert_eq!(
            image::load_from_memory(&sheet.bytes).unwrap().width(),
            CELL_WIDTH * 4
        );
        fixture.close().await;
    }
    #[tokio::test]
    async fn a_transparent_source_reads_as_a_photograph_on_neutral_grey_with_no_drawn_number() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
        // A transparent canonical figure must not land on black (a cut-out panel), and no
        // digit is drawn on the sheet for the plate model to copy.
        let picture = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([200, 30, 30, 0]),
        ));
        let mut bytes = Cursor::new(Vec::new());
        picture.write_to(&mut bytes, Container::Png).unwrap();
        let (sheet, legend) = cast_sheet(
            imagery,
            vec![CastIdentity {
                id: "hero:1".to_owned(),
                figure: fixture
                    .reference(ImageFormat::Png, Bytes::from(bytes.into_inner()))
                    .await,
            }],
        )
        .await
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
        fixture.close().await;
    }
    #[tokio::test]
    async fn duplicate_or_missing_cast_cannot_be_silently_omitted() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
        assert!(cast_sheet(imagery, vec![]).await.is_err());
        assert!(
            cast_sheet(
                imagery,
                vec![
                    identity(&fixture, "hero:1", 10).await,
                    identity(&fixture, "hero:1", 20).await
                ]
            )
            .await
            .is_err()
        );
        fixture.close().await;
    }
    #[tokio::test]
    async fn every_member_of_the_maximum_cast_keeps_a_visible_cell() {
        let fixture = TransformFixture::new().await;
        let mut cast = Vec::new();
        for slot in 1..=MAXIMUM_CAST {
            cast.push(identity(&fixture, &format!("person:{slot:02}"), (slot * 8) as u8).await);
        }
        let (sheet, legend) = cast_sheet(fixture.imagery(), cast).await.unwrap();
        let decoded = image::load_from_memory(&sheet.bytes).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (4 * CELL_WIDTH, 6 * CELL_HEIGHT));
        for slot in 0..MAXIMUM_CAST {
            let center_x = (slot as u32 % 4) * CELL_WIDTH + CELL_WIDTH / 2;
            let center_y = (slot as u32 / 4) * CELL_HEIGHT + CELL_HEIGHT / 2;
            assert!(decoded.get_pixel(center_x, center_y)[0].abs_diff(((slot + 1) * 8) as u8) < 12);
            assert!(legend.contains(&format!("slot {} = person:{:02}", slot + 1, slot + 1)));
        }
        fixture.close().await;
    }

    #[tokio::test]
    async fn reviewed_harbor_photographs_supply_npc_identities_and_refuse_stock_heroes() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
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
            let reference = store.load(source).unwrap();
            cast.push(CastIdentity {
                id: id.to_owned(),
                figure: fixture.reference(reference.format, reference.bytes).await,
            });
        }
        let (sheet, legend) = cast_sheet(imagery, cast).await.unwrap();
        assert!(legend.contains("npc:courier"));
        assert!(legend.contains("npc:bandit"));
        let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/plate-continuity");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("harbor-canonical-cast.jpg"), &sheet.bytes).unwrap();
        std::fs::write(output.join("harbor-canonical-cast.legend.txt"), legend).unwrap();
        fixture.close().await;
    }
    fn exif_six_jpeg() -> Vec<u8> {
        let pixels = image::RgbImage::from_fn(17, 23, |_, y| {
            if y < 11 {
                image::Rgb([0, 0, 230])
            } else {
                image::Rgb([230, 0, 0])
            }
        });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
            .encode_image(&pixels)
            .unwrap();
        let mut tagged = jpeg[..2].to_vec();
        tagged.extend_from_slice(&[0xff, 0xe1, 0, 0x22]);
        tagged.extend_from_slice(b"Exif\0\0");
        tagged.extend_from_slice(&[
            0x49, 0x49, 0x2a, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0,
            0,
        ]);
        tagged.extend_from_slice(&jpeg[2..]);
        tagged
    }

    #[tokio::test]
    async fn cast_photo_sizes_match_the_normalized_nonsquare_exif_six_picture() {
        let fixture = TransformFixture::new().await;
        let bytes = exif_six_jpeg();
        let normalized = image::load_from_memory_with_format(&bytes, Container::Jpeg)
            .unwrap()
            .rotate90();
        let mut png = Cursor::new(Vec::new());
        normalized.write_to(&mut png, Container::Png).unwrap();
        let source = fixture.reference(ImageFormat::Jpeg, bytes.into()).await;
        let normalized = fixture
            .reference(ImageFormat::Png, png.into_inner().into())
            .await;
        let (actual, _) = cast_sheet(
            fixture.imagery(),
            vec![CastIdentity {
                id: "hero-1".into(),
                figure: source,
            }],
        )
        .await
        .unwrap();
        let (expected, _) = cast_sheet(
            fixture.imagery(),
            vec![CastIdentity {
                id: "hero-1".into(),
                figure: normalized,
            }],
        )
        .await
        .unwrap();
        assert_eq!(actual.bytes, expected.bytes);
        let decoded = image::load_from_memory(&actual.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (512, 768));
        fixture.close().await;
    }
}
