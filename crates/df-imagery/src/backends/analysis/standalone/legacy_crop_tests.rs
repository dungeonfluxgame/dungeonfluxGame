//! Original plate/crop golden assertions, relocated to the private preparation boundary.
use super::{
    geometry::{SegmentedPerson, people_from_segments},
    pictures,
};
use crate::config::AnalysisPreparationLimits;
use image::{DynamicImage, RgbImage};
use std::io::Cursor;
const CROP_SIDE: u32 = 448;
#[derive(Debug, Eq, PartialEq)]
enum CropError {
    Decode,
}
#[derive(Debug, Eq, PartialEq)]
struct PreparedPlate {
    jpeg: Vec<u8>,
    width: u32,
    height: u32,
}
fn prepare_plate(bytes: &[u8]) -> Result<PreparedPlate, CropError> {
    let limits = AnalysisPreparationLimits {
        max_picture_bytes: 3 * 1024 * 1024,
        ..Default::default()
    };
    let (jpeg, width, height) =
        pictures::plate(bytes, &limits, 256 * 1024 * 1024, bytes.len() as u64)
            .map_err(|_| CropError::Decode)?;
    Ok(PreparedPlate {
        jpeg,
        width,
        height,
    })
}
fn candidate_crops(
    bytes: &[u8],
    people: &[&SegmentedPerson],
) -> Result<Vec<(String, Vec<u8>)>, CropError> {
    pictures::crops(
        bytes,
        people,
        &AnalysisPreparationLimits::default(),
        256 * 1024 * 1024,
        bytes.len() as u64,
    )
    .map_err(|_| CropError::Decode)
}
fn instance(
    score: f32,
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
) -> df_providers::SegmentInstance {
    df_providers::SegmentInstance {
        index: 0,
        score,
        mask: df_providers::MaskRows::from_runs(
            200,
            100,
            (top..bottom)
                .map(|row| df_providers::MaskRun {
                    start: row * 200 + left,
                    length: right - left,
                })
                .collect(),
        )
        .unwrap(),
    }
}
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
