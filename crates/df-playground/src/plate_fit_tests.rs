//! How a plate of any shape lies in the frame and where its scan's points land on the screen.

use df_scene_layout::{RegionGeometry, RegionId};

use crate::bubble_place::anchor_on_stage;
use crate::camera::{Framing, framing_within};
use crate::fixture::Camera;
use crate::image_layout::*;
use crate::lobby::model::{Bubble, BubbleTone};
use crate::plate_fit::{COVER_TOLERANCE_PERCENT, Fit, PlateFit};

/// The shapes the page is asked to show, as pixel sizes: square, 4:3, 3:2, 16:9, a plate a little
/// off 16:9, 21:9 and 9:16.
const SHAPES: [(u32, u32); 8] = [
    (512, 512),
    (1024, 768),
    (1536, 1024),
    (1920, 1080),
    (1664, 928),
    (1920, 1030),
    (2560, 1080),
    (720, 1280),
];

fn region(label: &str, head: (u16, u16), left: u16, right: u16) -> LayoutRegion {
    let geometry = RegionGeometry {
        left_permille: left,
        top_permille: head.1,
        right_permille: right,
        bottom_permille: 1000,
        head_x_permille: head.0,
        head_y_permille: head.1,
    };
    let id = RegionId::new(&format!("r{:08x}", head.0)).unwrap();
    LayoutRegion::new(id, geometry, 950)
        .unwrap()
        .with_label(Some(label.to_owned()))
}

fn layout_of(size: (u32, u32), regions: Vec<LayoutRegion>) -> ImageLayout {
    let picture = PictureBinding {
        name: "plate.webp".to_owned(),
        sha256: "ab".repeat(32),
        width_px: size.0,
        height_px: size.1,
    };
    ImageLayout::new(picture, "sam-3.1", "person", regions)
}

fn thinking(region: &str) -> Bubble {
    Bubble {
        speaker: "Danny".to_owned(),
        line: String::new(),
        emphasis: None,
        glyph: None,
        portrait: None,
        tone: BubbleTone::Thought,
        left_percent: 10,
        top_percent: 10,
        width_percent: 8,
        tail_x_percent: 14,
        tail_y_percent: 30,
        anchor: Some(BubbleAnchor {
            region: region.to_owned(),
            shift_percent: 0,
            point: CharacterPoint::Head,
        }),
    }
}

fn close(left: f32, right: f32) -> bool {
    (left - right).abs() < 0.002
}

#[test]
fn a_plate_near_the_frames_shape_is_cropped_to_fill_it_and_any_other_is_shown_whole() {
    let fit = |size: (u32, u32)| PlateFit::of_size(size.0, size.1).fit();
    assert_eq!(fit((1920, 1080)), Fit::Cover);
    assert_eq!(fit((1664, 928)), Fit::Cover);
    assert_eq!(fit((1672, 941)), Fit::Cover);
    assert_eq!(fit((1920, 1030)), Fit::Cover);
    assert_eq!(fit((512, 512)), Fit::Contain);
    assert_eq!(fit((1024, 768)), Fit::Contain);
    assert_eq!(fit((1536, 1024)), Fit::Contain);
    assert_eq!(fit((2560, 1080)), Fit::Contain);
    assert_eq!(fit((720, 1280)), Fit::Contain);
    assert_eq!(PlateFit::of_size(0, 0), PlateFit::FRAME);
    assert_eq!(COVER_TOLERANCE_PERCENT, 5.0);
}

#[test]
fn a_sixteen_by_nine_plate_is_the_frame_and_every_point_stays_where_it_was() {
    let fit = PlateFit::of_size(1920, 1080);
    assert_eq!(fit, PlateFit::FRAME);
    for x in [0, 1, 459, 500, 845, 999, 1000] {
        for y in [0, 158, 420, 1000] {
            assert_eq!(fit.stage_permille((x, y)), (x, y));
        }
    }
    assert_eq!(fit.stage_units((500, 1000)), (50.0, 56.25));
    assert_eq!(
        fit.box_style(),
        "left:0.000%;top:0.000%;width:100.000%;height:100.000%;"
    );
}

#[test]
fn a_plate_a_little_off_sixteen_by_nine_is_cropped_by_its_exact_amount() {
    // 1664 by 928 is 0.86 percent wider than the frame: the sides lose 0.43 percent each.
    let fit = PlateFit::of_size(1664, 928);
    let (centre_x, centre_y) = fit.stage_fraction((500, 500));
    assert!(close(centre_x, 0.5) && close(centre_y, 0.5));
    let (left, top) = fit.stage_fraction((0, 0));
    let (right, bottom) = fit.stage_fraction((1000, 1000));
    assert!(left < 0.0 && left > -0.005);
    assert!(close(top, 0.0) && close(bottom, 1.0));
    assert!(right > 1.0 && right < 1.005);
    // The old layout of 1672 by 941 put its points at the frame's: so they stay, to a thousandth.
    let old = PlateFit::of_size(1672, 941);
    for point in [(845, 420), (500, 200), (100, 970)] {
        assert_eq!(old.stage_permille(point), point);
    }
}

#[test]
fn a_square_plate_lies_in_the_middle_of_the_frame_at_the_frames_height() {
    let fit = PlateFit::of_size(512, 512);
    assert_eq!(fit.fit(), Fit::Contain);
    assert_eq!(
        fit.box_style(),
        "left:21.875%;top:0.000%;width:56.250%;height:100.000%;"
    );
    assert_eq!(fit.stage_permille((0, 0)), (219, 0));
    assert_eq!(fit.stage_permille((1000, 1000)), (781, 1000));
    assert_eq!(fit.stage_permille((500, 500)), (500, 500));
}

#[test]
fn a_tall_plate_is_a_narrow_column_and_a_wide_one_a_band() {
    let tall = PlateFit::of_size(720, 1280);
    assert!(close(tall.width(), 0.31640625));
    assert_eq!(tall.stage_permille((0, 500)).0, 342);
    let wide = PlateFit::of_size(2560, 1080);
    // 21:9 shown whole: the band is 75 percent of the frame's height, in the middle.
    assert_eq!(wide.stage_permille((500, 0)), (500, 125));
    assert_eq!(wide.stage_permille((500, 1000)), (500, 875));
}

#[test]
fn the_square_plates_head_in_the_evidence_gets_its_thinking_cue_above_it() {
    // Danny's free roam on Forge: 512 by 512, head at 45.9, 15.8 percent of the picture.
    let layout = layout_of((512, 512), vec![region("danny", (459, 158), 380, 540)]);
    let place = anchored_place(&thinking("danny"), Some(&layout)).unwrap();
    // 459 of the picture is 477 of the frame (0.21875 + 0.459 * 0.5625); the tip stops short of
    // the hair as before (15), and the height is the frame's own.
    assert_eq!(place.tip_x_permille, 477);
    assert_eq!(place.tip_y_permille, 158 - HEAD_GAP_PERMILLE);
    assert_eq!(place.bottom_permille, 1000 - 143);
    // The cue is centred over the head, not on the picture's 459 of the frame.
    assert_eq!(place.left_permille, 477 - 155);
}

/// Where the picture of `size` lies in a frame of `frame` pixels, worked out from first
/// principles: scaled to fill the frame (a shape within the tolerance of the frame's) or to lie
/// inside it, and centred. Returns its left, top, width and height.
fn drawn_box(size: (u32, u32), frame: (f32, f32)) -> (f32, f32, f32, f32) {
    let (picture_width, picture_height) = (size.0 as f32, size.1 as f32);
    let across = frame.0 / picture_width;
    let down = frame.1 / picture_height;
    let near = (picture_width / picture_height / (frame.0 / frame.1) - 1.0).abs() <= 0.05;
    let scale = if near {
        across.max(down)
    } else {
        across.min(down)
    };
    let (width, height) = (picture_width * scale, picture_height * scale);
    (
        (frame.0 - width) / 2.0,
        (frame.1 - height) / 2.0,
        width,
        height,
    )
}

#[test]
fn the_head_anchor_lands_on_the_head_for_every_shape_under_zoom_and_pan() {
    let head = (437, 211);
    let frame = (1600.0_f32, 900.0_f32);
    let framings = [
        Framing::WHOLE_STAGE,
        Framing {
            scale: 2.0,
            x_percent: -50.0,
            y_percent: -20.0,
        },
        Framing {
            scale: 1.12,
            x_percent: -4.0,
            y_percent: 0.0,
        },
    ];
    for size in SHAPES {
        let layout = layout_of(size, vec![region("danny", head, 300, 600)]);
        let place = anchored_place(&thinking("danny"), Some(&layout)).unwrap();
        let (left, top, width, height) = drawn_box(size, frame);
        // The tip stops HEAD_GAP of the frame's height short of the head.
        let head_at = (
            left + width * 0.437,
            top + height * 0.211 - frame.1 * f32::from(HEAD_GAP_PERMILLE) / 1000.0,
        );
        let tip = (place.tip_x_permille, place.tip_y_permille);
        for framing in framings {
            let (x, y) = anchor_on_stage(tip, &framing, frame.0, frame.1);
            let want_x = framing.x_percent / 100.0 * frame.0 + framing.scale * head_at.0;
            let want_y = framing.y_percent / 100.0 * frame.1 + framing.scale * head_at.1;
            // A thousandth of the frame is the page's own resolution.
            let slack = 1.0 + framing.scale * 1.6;
            assert!(
                (x - want_x).abs() < slack && (y - want_y).abs() < slack,
                "{size:?} {framing:?}: tip at ({x}, {y}), head at ({want_x}, {want_y})"
            );
        }
    }
}

#[test]
fn the_pictures_box_has_the_pictures_own_shape_and_holds_the_whole_picture_when_shown_whole() {
    let frame = (1600.0_f32, 900.0_f32);
    for size in SHAPES {
        let fit = PlateFit::of_size(size.0, size.1);
        let (left, top) = fit.stage_fraction((0, 0));
        let (right, bottom) = fit.stage_fraction((1000, 1000));
        let shape = (right - left) * frame.0 / ((bottom - top) * frame.1);
        let want = size.0 as f32 / size.1 as f32;
        assert!(
            (shape - want).abs() < 0.001 * want,
            "{size:?}: {shape} for {want}"
        );
        let (box_left, box_top, box_width, box_height) = drawn_box(size, frame);
        assert!(close(left, box_left / frame.0) && close(top, box_top / frame.1));
        assert!(close(right - left, box_width / frame.0));
        assert!(close(bottom - top, box_height / frame.1));
        match fit.fit() {
            Fit::Contain => {
                assert!(left >= -0.0001 && top >= -0.0001, "{size:?}");
                assert!(right <= 1.0001 && bottom <= 1.0001, "{size:?}");
            }
            Fit::Cover => {
                assert!(left <= 0.0001 && top <= 0.0001, "{size:?}");
                assert!(right >= 0.9999 && bottom >= 0.9999, "{size:?}");
            }
        }
    }
}

#[test]
fn no_named_region_is_cropped_away_whatever_the_shape() {
    // A person at each edge of the picture and one in the middle, heads near the top and bottom.
    let regions = vec![
        region("left", (20, 10), 0, 60),
        region("right", (980, 990), 940, 1000),
        region("middle", (500, 500), 450, 550),
    ];
    for size in SHAPES {
        let layout = layout_of(size, regions.clone());
        let fit = PlateFit::of_picture(layout.picture());
        for named in layout.regions() {
            let corners = [
                (named.left_permille(), named.top_permille()),
                (named.right_permille(), named.bottom_permille()),
                (named.head_x_permille(), named.head_y_permille()),
            ];
            for corner in corners {
                let (x, y) = fit.stage_fraction(corner);
                let lost = match fit.fit() {
                    // Shown whole: nothing at all is lost.
                    Fit::Contain => 0.0,
                    // Cropped within the tolerance: at most half of it from each side.
                    Fit::Cover => COVER_TOLERANCE_PERCENT as f32 / 200.0 + 0.001,
                };
                assert!(
                    x >= -lost && x <= 1.0 + lost && y >= -lost && y <= 1.0 + lost,
                    "{size:?}: {corner:?} of {:?} is cropped away at ({x}, {y})",
                    named.label()
                );
            }
        }
    }
}

#[test]
fn the_face_boxes_and_markers_follow_the_fit() {
    let fit = PlateFit::of_size(512, 512);
    assert_eq!(
        fit.stage_box_permille((400, 100, 500, 250)),
        (444, 100, 500, 250)
    );
    let layout = layout_of((512, 512), vec![region("danny", (459, 158), 380, 540)]);
    let marker = CharacterMarker {
        region: "danny".to_owned(),
        point: CharacterPoint::Head,
        text: "Danny".to_owned(),
        glyph: None,
        dx_percent: 2,
        dy_percent: -3,
    };
    assert_eq!(marker_place(&marker, Some(&layout)), Some((497, 128)));
}

#[test]
fn the_camera_aims_at_the_character_where_the_picture_puts_it() {
    let layout = layout_of((512, 512), vec![region("danny", (500, 300), 450, 550)]);
    let targets = region_target_elements(&layout);
    let face = targets
        .iter()
        .find(|element| element.id == "region:danny:upper")
        .unwrap();
    // The focus of the upper shot: the head across, 275 of the figure's 700 below it.
    assert!((face.x - 50.0).abs() < 0.01);
    assert!((face.y - 56.25 * 0.492).abs() < 0.01);
    let camera = Camera {
        target: Some(face.id.clone()),
        zoom_percent: 200,
        duration_ms: 0,
        serial: 1,
        began_ms: 0,
        elapsed_ms: 0,
    };
    let framing = framing_within(Some(&camera), &targets, 1000);
    // The target is in the middle of the screen.
    assert!(close(
        framing.x_percent / 100.0 + framing.scale * face.x / 100.0,
        0.5
    ));
    assert!(close(
        framing.y_percent / 100.0 + framing.scale * face.y / 56.25,
        0.5
    ));
}

#[test]
fn a_character_at_the_square_plates_left_edge_is_a_fifth_into_the_frame() {
    let layout = layout_of((512, 512), vec![region("edge", (10, 300), 0, 60)]);
    let targets = region_target_elements(&layout);
    let target = targets
        .iter()
        .find(|element| element.id == "region:edge:upper")
        .unwrap();
    // 0.21875 + 0.010 * 0.5625 of the frame, in canvas units: not the 1 percent of the picture.
    assert!((target.x - 22.4375).abs() < 0.01);
}
