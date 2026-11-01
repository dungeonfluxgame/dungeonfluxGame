//! Ordered party photographs and bounded reference preparation through ImageService.
//! Layout stays with gameplay; authorized pins and every pixel operation use typed transforms.

use df_imagery::{
    AlphaHandling, BlendMode, CanvasSpec, CompositeLayer, Dimensions, EncodedAlphaBlend,
    EncodedFilter, Encoding, ImageTransform, LossyQuality, Purpose, PurposedUse, RasterOutputSpec,
    Rgb, TransformSource, TransformSpec, VariantRole,
};
use df_providers::ImageFormat;
use image::ImageFormat as Container;

use super::{ImageFailure, Imagery, ReferenceImage};

const GREY: u8 = 128;
/// Neutral grey between photographs preserves their separate positional identities.
const GAP: u32 = 24;
const JPEG_QUALITY: u8 = 90;
/// Existing image crate default used by reference fitting, independent of sheet quality.
#[cfg(test)]
const FIT_JPEG_QUALITY: u8 = 75;
const FIGURE_HEIGHT: u32 = 1024;
const MAXIMUM_FIGURES: usize = 6;
const MAXIMUM_SHEET_WIDTH: u32 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum SheetError {
    FigureCount,
    Decode,
    Encode,
    Execution(ImageFailure),
}

pub(super) struct PictureHeader {
    pub(super) width_px: u32,
    pub(super) height_px: u32,
    pub(super) has_alpha: bool,
}

fn container_of(format: ImageFormat) -> Container {
    match format {
        ImageFormat::Png => Container::Png,
        ImageFormat::Jpeg => Container::Jpeg,
        ImageFormat::Webp => Container::WebP,
    }
}

/// Reads dimensions and channel metadata without decoding or changing pixels.
pub(super) fn picture_header(reference: &ReferenceImage) -> Result<PictureHeader, SheetError> {
    if image::guess_format(&reference.bytes).map_err(|_| SheetError::Decode)?
        != container_of(reference.format)
    {
        return Err(SheetError::Decode);
    }
    let header = df_imagery::source_header(&reference.bytes).map_err(|_| SheetError::Decode)?;
    Ok(PictureHeader {
        width_px: header.dimensions.width_px(),
        height_px: header.dimensions.height_px(),
        has_alpha: header.has_alpha,
    })
}

pub(super) fn grey() -> Rgb {
    Rgb {
        red: GREY,
        green: GREY,
        blue: GREY,
    }
}

pub(super) fn output(
    width_px: u32,
    height_px: u32,
    alpha: AlphaHandling,
    encoding: Encoding,
) -> Result<RasterOutputSpec, SheetError> {
    RasterOutputSpec::new(
        Dimensions::new(width_px, height_px).map_err(|_| SheetError::Encode)?,
        alpha,
        encoding,
    )
    .map_err(|_| SheetError::Encode)
}

pub(super) fn jpeg(quality: u8) -> Result<Encoding, SheetError> {
    Ok(Encoding::Jpeg {
        quality: LossyQuality::new(quality).map_err(|_| SheetError::Encode)?,
    })
}

pub(super) async fn transform_reference(
    imagery: &Imagery,
    reference: &ReferenceImage,
    steps: Vec<ImageTransform>,
    output: RasterOutputSpec,
    subject: &str,
) -> Result<ReferenceImage, SheetError> {
    let source = imagery
        .pin_reference(reference)
        .await
        .map_err(SheetError::Execution)?;
    imagery
        .transform(
            subject,
            PurposedUse {
                purpose: Purpose::ReferencePrep,
                role: VariantRole::Reference,
            },
            TransformSpec {
                source: TransformSource::Image(source),
                steps,
            },
            output,
        )
        .await
        .map_err(SheetError::Execution)
}

/// Composes all photographs in reading order, wrapping wide parties into rows of three.
pub(in crate::gameplay) async fn party_sheet(
    imagery: &Imagery,
    figures: &[ReferenceImage],
) -> Result<ReferenceImage, SheetError> {
    if figures.is_empty() || figures.len() > MAXIMUM_FIGURES {
        return Err(SheetError::FigureCount);
    }
    let headers = figures
        .iter()
        .map(picture_header)
        .collect::<Result<Vec<_>, _>>()?;
    let figure_count = u32::try_from(figures.len()).map_err(|_| SheetError::FigureCount)?;
    let row_width = headers
        .iter()
        .map(|header| {
            (u64::from(header.width_px) * u64::from(FIGURE_HEIGHT) / u64::from(header.height_px))
                .max(1)
        })
        .sum::<u64>()
        + u64::from(GAP) * u64::from(figure_count - 1);
    let columns = if row_width > u64::from(MAXIMUM_SHEET_WIDTH) {
        figures.len().min(3)
    } else {
        figures.len()
    };
    let columns_px = u32::try_from(columns).map_err(|_| SheetError::FigureCount)?;
    let rows_px =
        u32::try_from(figures.len().div_ceil(columns)).map_err(|_| SheetError::FigureCount)?;
    let maximum_width = (MAXIMUM_SHEET_WIDTH - GAP * (columns_px - 1)) / columns_px;
    let mut scaled = Vec::with_capacity(figures.len());
    for (figure, header) in figures.iter().zip(headers) {
        let width_px = u32::try_from(
            (u64::from(header.width_px) * u64::from(FIGURE_HEIGHT) / u64::from(header.height_px))
                .clamp(1, u64::from(maximum_width)),
        )
        .map_err(|_| SheetError::Encode)?;
        let height_px = u32::try_from(
            (u64::from(header.height_px) * u64::from(width_px) / u64::from(header.width_px))
                .clamp(1, u64::from(FIGURE_HEIGHT)),
        )
        .map_err(|_| SheetError::Encode)?;
        let dimensions = Dimensions::new(width_px, height_px).map_err(|_| SheetError::Encode)?;
        let picture = transform_reference(
            imagery,
            figure,
            vec![
                ImageTransform::EncodedMatte {
                    fill: grey(),
                    blend: EncodedAlphaBlend::IntegerFloor,
                },
                ImageTransform::EncodedResize {
                    dimensions,
                    filter: EncodedFilter::Triangle,
                },
            ],
            output(width_px, height_px, AlphaHandling::Opaque, Encoding::Png)?,
            "reference-prep:party-figure",
        )
        .await?;
        scaled.push((picture, width_px, height_px));
    }
    let width_px = scaled
        .chunks(columns)
        .map(|row| {
            // Every row contains at most the six validated figures.
            row.iter().map(|(_, width, _)| *width).sum::<u32>() + GAP * (row.len() as u32 - 1)
        })
        .max()
        .ok_or(SheetError::FigureCount)?;
    let height_px = rows_px * FIGURE_HEIGHT + (rows_px - 1) * GAP;
    let mut layers = Vec::with_capacity(scaled.len());
    for (row, pictures) in scaled.chunks(columns).enumerate() {
        let row_px = u32::try_from(row).map_err(|_| SheetError::Encode)?;
        let mut left_px = 0;
        for (picture, width, height) in pictures {
            let top_px = row_px * (FIGURE_HEIGHT + GAP) + (FIGURE_HEIGHT - height) / 2;
            let source = imagery
                .pin_reference(picture)
                .await
                .map_err(SheetError::Execution)?;
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
            left_px += width + GAP;
        }
    }
    imagery
        .transform(
            "reference-prep:party-sheet",
            PurposedUse {
                purpose: Purpose::ReferencePrep,
                role: VariantRole::Reference,
            },
            TransformSpec {
                source: TransformSource::Canvas(
                    CanvasSpec::new(width_px, height_px, grey()).map_err(|_| SheetError::Encode)?,
                ),
                steps: vec![ImageTransform::Composite(layers)],
            },
            output(
                width_px,
                height_px,
                AlphaHandling::Opaque,
                jpeg(JPEG_QUALITY)?,
            )?,
        )
        .await
        .map_err(SheetError::Execution)
}

/// The bounded JPEG the vision model receives, retaining the existing encoded-space grey matte.
#[cfg(test)]
async fn vision_picture(
    imagery: &Imagery,
    reference: &ReferenceImage,
    maximum_width: u32,
) -> Result<Vec<u8>, SheetError> {
    let header = picture_header(reference)?;
    let width_px = header.width_px.min(maximum_width);
    if width_px == 0 {
        return Err(SheetError::Decode);
    }
    let height_px = u32::try_from(
        u64::from(header.height_px) * u64::from(width_px) / u64::from(header.width_px),
    )
    .map_err(|_| SheetError::Decode)?
    .max(1);
    let dimensions = Dimensions::new(width_px, height_px).map_err(|_| SheetError::Encode)?;
    let image = transform_reference(
        imagery,
        reference,
        vec![
            ImageTransform::EncodedMatte {
                fill: grey(),
                blend: EncodedAlphaBlend::IntegerFloor,
            },
            ImageTransform::EncodedResize {
                dimensions,
                filter: EncodedFilter::Triangle,
            },
        ],
        output(width_px, height_px, AlphaHandling::Opaque, jpeg(80)?)?,
        "reference-prep:vision-picture",
    )
    .await?;
    Ok(image.bytes.to_vec())
}

/// Fits oversized references with the existing four-attempt size rule, retaining format and alpha.
#[cfg(test)]
async fn fit_reference(
    imagery: &Imagery,
    reference: &ReferenceImage,
    maximum_bytes: usize,
) -> Result<Option<ReferenceImage>, SheetError> {
    if reference.bytes.len() <= maximum_bytes {
        return Ok(None);
    }
    let header = picture_header(reference)?;
    let encoding = match reference.format {
        ImageFormat::Png => Encoding::Png,
        ImageFormat::Jpeg => jpeg(FIT_JPEG_QUALITY)?,
        ImageFormat::Webp => Encoding::WebP { quality: None },
    };
    let alpha = if header.has_alpha {
        AlphaHandling::PreserveAlpha
    } else {
        AlphaHandling::Opaque
    };
    let mut previous_bytes = reference.bytes.len();
    for _ in 0..4 {
        let ratio = (maximum_bytes as f64 / previous_bytes as f64).sqrt() * 0.9;
        // Oversized input gives ratio < 1, so these truncated source dimensions fit u32.
        let width_px = ((f64::from(header.width_px) * ratio) as u32).max(1);
        let height_px = ((f64::from(header.height_px) * ratio) as u32).max(1);
        let dimensions = Dimensions::new(width_px, height_px).map_err(|_| SheetError::Encode)?;
        let mut steps = vec![ImageTransform::EncodedResize {
            dimensions,
            filter: EncodedFilter::Triangle,
        }];
        if !header.has_alpha {
            steps.push(ImageTransform::DiscardAlpha);
        }
        let image = transform_reference(
            imagery,
            reference,
            steps,
            output(width_px, height_px, alpha, encoding)?,
            "reference-prep:fit-reference",
        )
        .await?;
        if image.bytes.len() <= maximum_bytes {
            return Ok(Some(image));
        }
        previous_bytes = image.bytes.len().max(previous_bytes);
    }
    Err(SheetError::Encode)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::TransformFixture;
    use super::*;
    use bytes::Bytes;
    use image::DynamicImage;
    use std::io::Cursor;

    async fn figure(
        fixture: &TransformFixture,
        width: u32,
        height: u32,
        alpha: u8,
        color: [u8; 3],
    ) -> ReferenceImage {
        let mut image = image::RgbaImage::new(width, height);
        for pixel in image.pixels_mut() {
            *pixel = image::Rgba([color[0], color[1], color[2], alpha]);
        }
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), Container::Png)
            .unwrap();
        fixture
            .reference(ImageFormat::Png, Bytes::from(bytes))
            .await
    }

    #[tokio::test]
    async fn six_figures_wrap_to_two_rows_of_three_without_exceeding_the_sheet_bound() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
        let colors = [
            [200, 0, 0],
            [0, 200, 0],
            [0, 0, 200],
            [200, 200, 0],
            [0, 200, 200],
            [200, 0, 200],
        ];
        let mut figures = Vec::new();
        for color in colors {
            figures.push(figure(&fixture, 32, 32, 255, color).await);
        }
        let sheet = party_sheet(imagery, &figures).await.unwrap();
        let decoded = image::load_from_memory(&sheet.bytes).unwrap().to_rgb8();
        assert_eq!(decoded.width(), 3 * FIGURE_HEIGHT + 2 * GAP);
        assert_eq!(decoded.height(), 2 * FIGURE_HEIGHT + GAP);
        assert!(decoded.width() <= MAXIMUM_SHEET_WIDTH);
        for (index, expected) in colors.iter().enumerate() {
            let x = index as u32 % 3 * (FIGURE_HEIGHT + GAP) + FIGURE_HEIGHT / 2;
            let y = index as u32 / 3 * (FIGURE_HEIGHT + GAP) + FIGURE_HEIGHT / 2;
            let pixel = decoded.get_pixel(x, y).0;
            assert!(
                pixel
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| actual.abs_diff(*expected) < 12),
                "figure {index}: {pixel:?}"
            );
        }
        let wide = vec![figure(&fixture, 100, 10, 255, [40, 80, 160]).await; 6];
        let wide = party_sheet(imagery, &wide).await.unwrap();
        let decoded = image::load_from_memory(&wide.bytes).unwrap();
        assert!(decoded.width() <= MAXIMUM_SHEET_WIDTH);
        assert_eq!(decoded.height(), 2 * FIGURE_HEIGHT + GAP);
        fixture.close().await;
    }

    #[tokio::test]
    async fn six_narrow_figures_keep_one_row_when_it_fits() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
        let figures = vec![figure(&fixture, 10, 20, 255, [40, 80, 160]).await; 6];
        let sheet = party_sheet(imagery, &figures).await.unwrap();
        let decoded = image::load_from_memory(&sheet.bytes).unwrap();
        assert_eq!(decoded.width(), 6 * 512 + 5 * GAP);
        assert_eq!(decoded.height(), FIGURE_HEIGHT);
        fixture.close().await;
    }

    #[tokio::test]
    async fn figures_stand_side_by_side_on_grey_in_the_order_given() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
        let sheet = party_sheet(
            imagery,
            &[
                figure(&fixture, 100, 200, 255, [200, 0, 0]).await,
                figure(&fixture, 100, 200, 0, [0, 200, 0]).await,
            ],
        )
        .await
        .unwrap();
        assert_eq!(sheet.format, ImageFormat::Jpeg);
        let decoded = image::load_from_memory_with_format(&sheet.bytes, Container::Jpeg)
            .unwrap()
            .to_rgb8();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (1024 + GAP, FIGURE_HEIGHT)
        );
        let left = decoded.get_pixel(256, 512).0;
        let gap = decoded.get_pixel(512 + GAP / 2, 512).0;
        assert!(
            gap.iter().all(|channel| channel.abs_diff(GREY) < 12),
            "separate photographs are divided by neutral grey: {gap:?}"
        );
        let right = decoded.get_pixel(512 + GAP + 256, 512).0;
        assert!(
            left[0] > 150 && left[1] < 60,
            "the first figure is red: {left:?}"
        );
        assert!(
            right.iter().all(|channel| channel.abs_diff(GREY) < 12),
            "a fully transparent figure leaves grey: {right:?}"
        );
        fixture.close().await;
    }

    #[tokio::test]
    async fn a_plate_is_shown_to_the_vision_model_as_a_small_jpeg() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
        let big = figure(&fixture, 2560, 1440, 255, [30, 40, 50]).await;
        let small = vision_picture(imagery, &big, 1024).await.unwrap();
        let decoded = image::load_from_memory_with_format(&small, Container::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1024, 576));
        assert_eq!(
            vision_picture(
                imagery,
                &ReferenceImage::new(ImageFormat::Png, Bytes::from_static(b"not an image")),
                1024
            )
            .await,
            Err(SheetError::Decode)
        );
        fixture.close().await;
    }

    #[tokio::test]
    async fn the_same_figures_give_the_same_bytes() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
        let figures = [figure(&fixture, 80, 160, 255, [10, 20, 30]).await];
        assert_eq!(
            party_sheet(imagery, &figures).await.unwrap().sha256,
            party_sheet(imagery, &figures).await.unwrap().sha256
        );
        fixture.close().await;
    }

    #[tokio::test]
    async fn no_figures_too_many_figures_and_undecodable_figures_are_refused() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
        assert_eq!(
            party_sheet(imagery, &[]).await.unwrap_err(),
            SheetError::FigureCount
        );
        let one = figure(&fixture, 10, 20, 255, [1, 2, 3]).await;
        assert_eq!(
            party_sheet(imagery, &vec![one; 7]).await.unwrap_err(),
            SheetError::FigureCount
        );
        let broken = ReferenceImage::new(ImageFormat::Png, Bytes::from_static(b"not an image"));
        assert_eq!(
            party_sheet(imagery, &[broken]).await.unwrap_err(),
            SheetError::Decode
        );
        fixture.close().await;
    }

    #[tokio::test]
    async fn an_oversized_transparent_reference_is_sent_smaller_with_its_alpha() {
        let fixture = TransformFixture::new().await;
        let imagery = fixture.imagery();
        // Noise compresses badly, so a modest picture is already over a small limit.
        let mut noisy = image::RgbaImage::new(400, 600);
        for (index, pixel) in noisy.pixels_mut().enumerate() {
            let value = (index.wrapping_mul(2_654_435_761) >> 7) as u8;
            *pixel = image::Rgba([value, value.wrapping_mul(3), value.wrapping_mul(7), value]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(noisy)
            .write_to(&mut Cursor::new(&mut png), Container::Png)
            .unwrap();
        let limit = png.len() / 2;
        let reference = fixture
            .reference(ImageFormat::Png, Bytes::from(png.clone()))
            .await;
        assert!(
            fit_reference(imagery, &reference, png.len())
                .await
                .unwrap()
                .is_none()
        );
        let fitted = fit_reference(imagery, &reference, limit)
            .await
            .unwrap()
            .expect("an oversized reference is re-encoded");
        assert!(fitted.bytes.len() <= limit);
        let decoded = image::load_from_memory_with_format(&fitted.bytes, Container::Png).unwrap();
        assert!(decoded.color().has_alpha());
        assert!(decoded.width() < 400 && decoded.height() < 600);
        fixture.close().await;
    }
}
