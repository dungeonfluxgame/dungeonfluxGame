//! The pictures the segmentation and the vision judges are shown: the plate prepared for SAM,
//! and a numbered upper-body crop of each person SAM found.

use std::io::Cursor;

use image::{RgbImage, codecs::jpeg::JpegEncoder, imageops};

use super::{candidate_label, geometry::SegmentedPerson};

/// The plate is sent to SAM at its own size up to this long side (plates are 2560 by 1440).
const MAXIMUM_PLATE_SIDE: u32 = 2560;
/// The plate's JPEG must fit the adapter's request bound (3 MiB).
const MAXIMUM_PLATE_BYTES: usize = 3 * 1024 * 1024;
const PLATE_QUALITY: u8 = 90;
/// A candidate crop is at most this many pixels on its long side.
const CROP_SIDE: u32 = 448;
const CROP_QUALITY: u8 = 85;
/// A crop reaches this many head heights below the chin (shoulders and chest show which way a
/// person faces); with no head box it is the top share of the body.
const HEADS_BELOW_CHIN: f32 = 1.5;
const BODY_SHARE_WITHOUT_HEAD: f32 = 0.4;
const SIDE_MARGIN: f32 = 0.08;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum CropError {
    /// The bytes are not a picture the server can decode.
    Decode,
    Encode,
    /// The prepared plate is over the request bound.
    TooLarge,
}

/// The plate as SAM is sent it: a JPEG with the size its masks will have.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct PreparedPlate {
    pub(in crate::gameplay) jpeg: Vec<u8>,
    pub(in crate::gameplay) width: u32,
    pub(in crate::gameplay) height: u32,
}

fn encode_jpeg(picture: &RgbImage, quality: u8) -> Result<Vec<u8>, CropError> {
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut Cursor::new(&mut encoded), quality)
        .encode_image(picture)
        .map_err(|_| CropError::Encode)?;
    Ok(encoded)
}

/// Decodes the cached plate (JPEG, WebP or PNG) and encodes it as the JPEG SAM is sent,
/// scaled down only when its long side is over 2560 pixels.
pub(in crate::gameplay) fn prepare_plate(bytes: &[u8]) -> Result<PreparedPlate, CropError> {
    let decoded = image::load_from_memory(bytes).map_err(|_| CropError::Decode)?;
    let picture = decoded.to_rgb8();
    let (width, height) = (picture.width(), picture.height());
    if width == 0 || height == 0 {
        return Err(CropError::Decode);
    }
    let long = width.max(height);
    let picture = if long > MAXIMUM_PLATE_SIDE {
        let scale = f64::from(MAXIMUM_PLATE_SIDE) / f64::from(long);
        let scaled = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
        imageops::resize(
            &picture,
            scaled(width),
            scaled(height),
            imageops::FilterType::Triangle,
        )
    } else {
        picture
    };
    let jpeg = encode_jpeg(&picture, PLATE_QUALITY)?;
    if jpeg.len() > MAXIMUM_PLATE_BYTES {
        return Err(CropError::TooLarge);
    }
    Ok(PreparedPlate {
        jpeg,
        width: picture.width(),
        height: picture.height(),
    })
}

/// The part of the plate a judge sees for `person`, in fractions: the body's width with a small
/// margin, from the top of the body down to the shoulders and chest.
fn crop_region(person: &SegmentedPerson) -> (f32, f32, f32, f32) {
    let body = &person.body;
    let margin = (body.right - body.left) * SIDE_MARGIN;
    let bottom = match person.head {
        Some(head) => (head.bottom + (head.bottom - head.top) * HEADS_BELOW_CHIN).min(body.bottom),
        None => body.top + (body.bottom - body.top) * BODY_SHARE_WITHOUT_HEAD,
    };
    (
        (body.left - margin).max(0.0),
        body.top,
        (body.right + margin).min(1.0),
        bottom.max(body.top),
    )
}

/// One upper-body JPEG crop per person, labelled `cand-<n>` after the person's candidate number,
/// in the order given.
pub(in crate::gameplay) fn candidate_crops(
    plate: &[u8],
    people: &[&SegmentedPerson],
) -> Result<Vec<(String, Vec<u8>)>, CropError> {
    let picture: RgbImage = image::load_from_memory(plate)
        .map_err(|_| CropError::Decode)?
        .to_rgb8();
    let (width, height) = (picture.width() as f32, picture.height() as f32);
    people
        .iter()
        .map(|person| {
            let (left, top, right, bottom) = crop_region(person);
            let x = ((left * width).floor() as u32).min(picture.width() - 1);
            let y = ((top * height).floor() as u32).min(picture.height() - 1);
            let crop_width = (((right - left) * width).ceil() as u32).clamp(1, picture.width() - x);
            let crop_height =
                (((bottom - top) * height).ceil() as u32).clamp(1, picture.height() - y);
            let cropped = imageops::crop_imm(&picture, x, y, crop_width, crop_height).to_image();
            let long = cropped.width().max(cropped.height());
            let shown = if long > CROP_SIDE {
                let scale = f64::from(CROP_SIDE) / f64::from(long);
                let scaled = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
                imageops::resize(
                    &cropped,
                    scaled(cropped.width()),
                    scaled(cropped.height()),
                    imageops::FilterType::Triangle,
                )
            } else {
                cropped
            };
            Ok((
                candidate_label(person.candidate),
                encode_jpeg(&shown, CROP_QUALITY)?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::geometry::{people_from_segments, tests::instance};
    use super::*;
    use image::DynamicImage;

    fn picture(width: u32, height: u32) -> Vec<u8> {
        let image = RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
        });
        let mut png = Vec::new();
        DynamicImage::ImageRgb8(image)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png
    }

    #[test]
    fn a_plate_is_sent_as_a_jpeg_at_its_own_size() {
        let prepared = prepare_plate(&picture(640, 360)).unwrap();
        assert_eq!((prepared.width, prepared.height), (640, 360));
        let decoded = image::load_from_memory(&prepared.jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (640, 360));
    }

    #[test]
    fn a_plate_longer_than_the_bound_is_scaled_down_keeping_its_shape() {
        let prepared = prepare_plate(&picture(3200, 1800)).unwrap();
        assert_eq!((prepared.width, prepared.height), (2560, 1440));
    }

    #[test]
    fn bytes_that_are_not_a_picture_are_refused() {
        assert_eq!(prepare_plate(b"not a picture"), Err(CropError::Decode));
        assert_eq!(
            candidate_crops(b"not a picture", &[]),
            Err(CropError::Decode)
        );
    }

    #[test]
    fn each_person_gets_a_crop_of_the_head_and_shoulders_labelled_by_candidate() {
        // A 200 by 100 mask space over a 400 by 200 picture: masks are fractions of the plate.
        let people = people_from_segments(
            &[
                instance(0.95, 20, 10, 80, 100),
                instance(0.9, 120, 10, 180, 100),
            ],
            &[
                instance(0.9, 40, 10, 60, 30),
                instance(0.9, 140, 10, 160, 30),
            ],
        );
        let refs: Vec<&SegmentedPerson> = people.iter().collect();
        let crops = candidate_crops(&picture(400, 200), &refs).unwrap();
        assert_eq!(crops.len(), 2);
        assert_eq!(crops[0].0, "cand-1");
        assert_eq!(crops[1].0, "cand-2");
        let first = image::load_from_memory(&crops[0].1).unwrap();
        // Body 0.1..0.4 wide plus a margin of 8% each side: about 140 of 400 pixels; head
        // bottom 0.3 plus 1.5 heads of 0.2 is 0.6 of the height, under the body bottom, so
        // 0.1..0.6: 100 of 200.
        assert!((135..=145).contains(&first.width()), "{}", first.width());
        assert!((95..=105).contains(&first.height()), "{}", first.height());
    }

    #[test]
    fn a_person_without_a_head_is_cropped_to_the_top_of_the_body() {
        let people = people_from_segments(&[instance(0.95, 20, 10, 80, 100)], &[]);
        let crops = candidate_crops(&picture(400, 200), &[&people[0]]).unwrap();
        let shown = image::load_from_memory(&crops[0].1).unwrap();
        // The top 40% of a body 0.9 tall is 0.36 of the plate: 72 of 200 pixels.
        assert!((68..=76).contains(&shown.height()), "{}", shown.height());
    }

    #[test]
    fn a_large_crop_is_scaled_to_the_judge_size() {
        let people = people_from_segments(&[instance(0.95, 0, 0, 200, 100)], &[]);
        let crops = candidate_crops(&picture(2000, 1000), &[&people[0]]).unwrap();
        let shown = image::load_from_memory(&crops[0].1).unwrap();
        assert_eq!(shown.width().max(shown.height()), CROP_SIDE);
    }
}
