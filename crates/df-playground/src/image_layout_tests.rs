//! The playground's own placement over a scene layout. The record, points, shots, targets and cast
//! are tested in `df-scene-layout`.

use df_scene_layout::{RegionGeometry, RegionId};

use crate::image_layout::*;
use crate::lobby::model::{Bubble, BubbleTone, TAIL_INSET_PERCENT};

fn region(label: Option<&str>, head_x: u16, head_y: u16) -> LayoutRegion {
    let geometry = RegionGeometry {
        left_permille: head_x.saturating_sub(80),
        top_permille: head_y,
        right_permille: (head_x + 80).min(1000),
        bottom_permille: 1000,
        head_x_permille: head_x,
        head_y_permille: head_y,
    };
    let id = RegionId::new(&format!("r{head_x:08x}")).unwrap();
    LayoutRegion::new(id, geometry, 950)
        .unwrap()
        .with_label(label.map(str::to_owned))
}

fn layout(regions: Vec<LayoutRegion>) -> ImageLayout {
    let picture = PictureBinding {
        name: "plate.webp".to_owned(),
        sha256: "ab".repeat(32),
        width_px: 1672,
        height_px: 941,
    };
    ImageLayout::new(picture, "sam-3.1", "person", regions)
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
    assert_eq!((place.width_permille, place.left_permille), (310, 690));
    // The tip stops HEAD_GAP above the head at 420: 1000 - 405.
    assert_eq!(place.bottom_permille, 595);
    assert_eq!((place.tip_x_permille, place.tip_y_permille), (845, 405));
    assert_eq!((place.root_percent, place.tip_percent), (50, 50));
}

#[test]
fn a_shift_moves_the_box_and_the_trail_leans_back_to_the_point() {
    let place = anchored_place(
        &bubble(anchor("innkeeper", -3, CharacterPoint::Head)),
        Some(&scene()),
    )
    .unwrap();
    assert_eq!(place.left_permille, 660);
    assert_eq!((place.root_percent, place.tip_percent), (59, 59));
}

#[test]
fn the_box_stays_on_screen_and_another_point_is_touched_without_a_gap() {
    let edge = layout(vec![region(Some("edge"), 1000, 400)]);
    let place = anchored_place(
        &bubble(anchor("edge", 0, CharacterPoint::Head)),
        Some(&edge),
    )
    .unwrap();
    assert_eq!(place.left_permille, 690);
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
    // Two named characters, each with a target for every shot (the speaker push-in among them)
    // and one for the figure.
    assert_eq!(elements.len(), 10);
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

fn masked(label: &str, head_x: u16) -> LayoutRegion {
    region(Some(label), head_x, 300).with_mask(Some(format!("data:image/png;base64,{label}")))
}

fn speaking(region: &str, tone: BubbleTone) -> Bubble {
    Bubble {
        tone,
        line: "Hold.".to_owned(),
        ..bubble(anchor(region, 0, CharacterPoint::Head))
    }
}

#[test]
fn the_speaker_with_a_mask_is_outlined_once_and_a_thinker_is_not() {
    let layout = layout(vec![
        masked("innkeeper", 300),
        masked("hero", 600),
        region(Some("guard"), 800, 300),
    ]);
    let bubbles = vec![
        speaking("innkeeper", BubbleTone::Npc),
        speaking("hero", BubbleTone::Thought),
        speaking("innkeeper", BubbleTone::Npc),
        speaking("guard", BubbleTone::Npc),
        speaking("nobody", BubbleTone::Npc),
        bubble(None),
    ];
    assert_eq!(
        speaker_masks(&bubbles, Some(&layout)),
        vec![("innkeeper", "data:image/png;base64,innkeeper")]
    );
    assert!(speaker_masks(&bubbles, None).is_empty());
    assert!(speaker_masks(&[], Some(&layout)).is_empty());
}
