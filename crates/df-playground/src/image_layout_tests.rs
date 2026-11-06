//! The playground's own placement over a scene layout. The record, points, shots, targets and cast
//! are tested in `df-scene-layout`.

use crate::image_layout::*;
use crate::lobby::model::{Bubble, BubbleTone, TAIL_INSET_PERCENT};

fn region(label: Option<&str>, head_x: u16, head_y: u16) -> LayoutRegion {
    LayoutRegion {
        id: format!("r{head_x:08x}"),
        label: label.map(str::to_owned),
        left_permille: head_x.saturating_sub(80),
        top_permille: head_y,
        right_permille: (head_x + 80).min(1000),
        bottom_permille: 1000,
        head_x_permille: head_x,
        head_y_permille: head_y,
        score_permille: 950,
        mask: None,
    }
}

fn layout(regions: Vec<LayoutRegion>) -> ImageLayout {
    ImageLayout {
        schema: SCHEMA_VERSION,
        picture: PictureBinding {
            name: "plate.webp".to_owned(),
            sha256: "ab".repeat(32),
            width_px: 1672,
            height_px: 941,
        },
        model: "sam-3.1".to_owned(),
        prompt: "person".to_owned(),
        regions,
    }
}

fn scene() -> ImageLayout {
    layout(vec![
        region(None, 100, 300),
        region(Some("innkeeper"), 845, 420),
        region(Some("hero"), 500, 200),
    ])
}

fn bubble(anchor: Option<BubbleAnchor>) -> Bubble {
    Bubble {
        speaker: "Innkeeper".to_owned(),
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
        anchor,
    }
}

fn anchor(region: &str, shift_percent: i8, point: CharacterPoint) -> Option<BubbleAnchor> {
    Some(BubbleAnchor {
        region: region.to_owned(),
        shift_percent,
        point,
    })
}

#[test]
fn an_anchored_bubble_sits_over_the_head_with_its_tip_just_above_it() {
    let place = anchored_place(
        &bubble(anchor("innkeeper", 0, CharacterPoint::Head)),
        Some(&scene()),
    )
    .unwrap();
    assert_eq!((place.width_permille, place.left_permille), (80, 805));
    // The tip stops HEAD_GAP above the head at 420: 1000 - 405.
    assert_eq!(place.bottom_permille, 595);
    assert_eq!((place.root_percent, place.tip_percent), (50, 50));
}

#[test]
fn a_shift_moves_the_box_and_the_trail_leans_back_to_the_point() {
    let place = anchored_place(
        &bubble(anchor("innkeeper", -3, CharacterPoint::Head)),
        Some(&scene()),
    )
    .unwrap();
    assert_eq!(place.left_permille, 775);
    assert_eq!((place.root_percent, place.tip_percent), (87, 87));
}

#[test]
fn the_box_stays_on_screen_and_another_point_is_touched_without_a_gap() {
    let edge = layout(vec![region(Some("edge"), 1000, 400)]);
    let place = anchored_place(
        &bubble(anchor("edge", 0, CharacterPoint::Head)),
        Some(&edge),
    )
    .unwrap();
    assert_eq!(place.left_permille, 920);
    assert_eq!(place.root_percent, 100 - TAIL_INSET_PERCENT);
    assert_eq!(place.tip_percent, 100);
    let at_face = anchored_place(
        &bubble(anchor("hero", 0, CharacterPoint::Face)),
        Some(&scene()),
    )
    .unwrap();
    // The hero's face is at 288, with no gap: 1000 - 288.
    assert_eq!(at_face.bottom_permille, 712);
}

#[test]
fn without_an_anchor_a_layout_or_the_name_the_bubble_keeps_its_place() {
    assert_eq!(anchored_place(&bubble(None), Some(&scene())), None);
    assert_eq!(
        anchored_place(&bubble(anchor("innkeeper", 0, CharacterPoint::Head)), None),
        None
    );
    assert_eq!(
        anchored_place(
            &bubble(anchor("bard", 0, CharacterPoint::Head)),
            Some(&scene())
        ),
        None
    );
}

#[test]
fn a_marker_sits_at_its_point_moved_by_its_offset() {
    let marker = CharacterMarker {
        region: "hero".to_owned(),
        point: CharacterPoint::Feet,
        text: "Hero".to_owned(),
        glyph: None,
        dx_percent: 2,
        dy_percent: -3,
    };
    assert_eq!(marker_place(&marker, Some(&scene())), Some((520, 970)));
    let lost = CharacterMarker {
        region: "nobody".to_owned(),
        ..marker
    };
    assert_eq!(marker_place(&lost, Some(&scene())), None);
}

#[test]
fn camera_targets_become_stage_positions_for_named_characters_only() {
    let elements = region_target_elements(&scene());
    assert_eq!(elements.len(), 8);
    let hero = elements
        .iter()
        .find(|element| element.id == "region:hero:face")
        .unwrap();
    assert!((hero.x - 50.0).abs() < 0.01);
    assert!((hero.y - 18.0).abs() < 0.01);
}

#[test]
fn the_page_and_the_layout_crate_agree_on_the_zoom_range() {
    assert_eq!(
        df_scene_layout::MINIMUM_ZOOM_PERCENT,
        crate::limits::MINIMUM_ZOOM_PERCENT
    );
    assert_eq!(
        df_scene_layout::MAXIMUM_ZOOM_PERCENT,
        crate::limits::MAXIMUM_ZOOM_PERCENT
    );
}
