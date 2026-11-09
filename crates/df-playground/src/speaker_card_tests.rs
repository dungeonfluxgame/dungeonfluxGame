//! Which speakers get a card, what it shows, and how a face is cut from the room picture.

use df_scene_layout::{RegionGeometry, RegionId};

use crate::image_layout::*;
use crate::lobby::model::{Bubble, BubbleTone};
use crate::speaker_card::*;

fn region(label: &str, shown: &str, head_x: u16) -> LayoutRegion {
    let geometry = RegionGeometry {
        left_permille: head_x - 80,
        top_permille: 200,
        right_permille: head_x + 80,
        bottom_permille: 1000,
        head_x_permille: head_x,
        head_y_permille: 220,
    };
    let id = RegionId::new(&format!("r{head_x:08x}")).unwrap();
    LayoutRegion::new(id, geometry, 950)
        .unwrap()
        .with_label(Some(label.to_owned()))
        .with_display_name(Some(shown.to_owned()))
}

fn layout(name: &str, regions: Vec<LayoutRegion>) -> ImageLayout {
    let picture = PictureBinding {
        name: name.to_owned(),
        sha256: "ab".repeat(32),
        width_px: 1600,
        height_px: 900,
    };
    ImageLayout::new(picture, "sam-3.1", "person", regions)
}

fn room() -> ImageLayout {
    layout(
        "room.webp",
        vec![
            region("village_torchbearer", "Teren", 300),
            region("innkeeper", "Brann", 700),
        ],
    )
}

fn bubble(character: &str, tone: BubbleTone, line: &str) -> Bubble {
    Bubble {
        speaker: character.to_owned(),
        line: line.to_owned(),
        emphasis: None,
        glyph: None,
        portrait: None,
        tone,
        left_percent: 20,
        top_percent: 12,
        width_percent: 22,
        tail_x_percent: 30,
        tail_y_percent: 40,
        anchor: Some(BubbleAnchor {
            region: character.to_owned(),
            shift_percent: 0,
            point: CharacterPoint::Head,
        }),
    }
}

#[test]
fn a_speaker_in_the_picture_keeps_the_pointing_bubble() {
    let room = room();
    let bubbles = [bubble("innkeeper", BubbleTone::Npc, "Welcome.")];
    assert_eq!(speaker_card(&bubbles, Some(&room), Some(&room)), None);
}

#[test]
fn a_speaker_missing_from_the_picture_gets_a_card_with_a_face_and_a_shown_name() {
    let room = room();
    let close_up = None; // the close-up has no scan of its own
    let bubbles = [bubble("village_torchbearer", BubbleTone::Npc, "Look here.")];
    let card = speaker_card(&bubbles, close_up, Some(&room)).expect("a card");
    assert_eq!(card.name, "Teren", "a name, never the id");
    assert_eq!(card.line.as_deref(), Some("Look here."));
    let portrait = card.portrait.expect("the room picture gives a face");
    assert_eq!(portrait.media, "room.webp");
}

#[test]
fn a_speaker_the_layout_does_not_name_is_a_card_without_a_face() {
    let room = room();
    let bubbles = [bubble("stranger", BubbleTone::Npc, "Hm.")];
    let card = speaker_card(&bubbles, Some(&room), Some(&room)).expect("a card");
    assert_eq!(card.portrait, None);
    assert_eq!(
        card.name, "stranger",
        "the written speaker when nothing names them"
    );
}

#[test]
fn without_any_layout_the_card_has_a_name_and_a_line_only() {
    let bubbles = [bubble("innkeeper", BubbleTone::Npc, "Hm.")];
    let card = speaker_card(&bubbles, None, None).expect("a card");
    assert_eq!(card.portrait, None);
    assert_eq!(card.line.as_deref(), Some("Hm."));
}

#[test]
fn names_resolve_on_a_close_up_from_the_cast_layout() {
    let room = room();
    let bubble = bubble("village_torchbearer", BubbleTone::Npc, "Hm.");
    assert_eq!(bubble_name(&bubble, Some(&room)), "Teren");
    assert_eq!(bubble_name(&bubble, None), "village_torchbearer");
}

#[test]
fn an_off_screen_thinker_is_a_card_with_an_indicator() {
    let room = room();
    let bubbles = [bubble("innkeeper", BubbleTone::Thought, "")];
    let card = speaker_card(&bubbles, None, Some(&room)).expect("a card");
    assert_eq!(card.line, None);
    assert_eq!(card.name, "Brann");
    assert!(card.card_identity().ends_with("-thinking"));
}

#[test]
fn one_card_at_a_time_the_latest_speaker_over_a_thinker() {
    let room = room();
    let bubbles = [
        bubble("village_torchbearer", BubbleTone::Npc, "First."),
        bubble("innkeeper", BubbleTone::Npc, "Second."),
        bubble("stranger", BubbleTone::Thought, ""),
    ];
    let card = speaker_card(&bubbles, None, Some(&room)).expect("a card");
    assert_eq!(card.character, "innkeeper");
}

#[test]
fn a_bubble_without_an_anchor_never_makes_a_card() {
    let mut free = bubble("innkeeper", BubbleTone::Npc, "Hi.");
    free.anchor = None;
    assert_eq!(speaker_card(&[free], None, None), None);
}

#[test]
fn a_card_key_changes_with_the_line_and_the_speaker() {
    let room = room();
    let key = |character: &str, line: &str| {
        speaker_card(
            &[bubble(character, BubbleTone::Npc, line)],
            None,
            Some(&room),
        )
        .expect("a card")
        .card_identity()
    };
    assert_eq!(key("innkeeper", "One."), key("innkeeper", "One."));
    assert_ne!(key("innkeeper", "One."), key("innkeeper", "Two."));
    assert_ne!(key("innkeeper", "One."), key("village_torchbearer", "One."));
}

#[test]
fn the_portrait_is_a_square_in_pixels_centred_on_the_face_and_inside_the_picture() {
    let room = room();
    let picture = room.picture();
    let region = room.region("innkeeper").expect("a region");
    let crop = portrait_crop(region, picture);
    let width_px = u32::from(crop.width_permille) * picture.width_px / 1000;
    let height_px = u32::from(crop.height_permille) * picture.height_px / 1000;
    assert!(
        width_px.abs_diff(height_px) <= 2,
        "{width_px} by {height_px}"
    );
    assert!(u32::from(crop.left_permille + crop.width_permille) <= 1000);
    assert!(u32::from(crop.top_permille + crop.height_permille) <= 1000);
    let centre = crop.left_permille + crop.width_permille / 2;
    assert!(
        centre.abs_diff(700) < 60,
        "centred on the head at 700, got {centre}"
    );
}

#[test]
fn a_head_at_the_picture_edge_is_cut_inside_the_picture() {
    let edge = layout("edge.webp", vec![region("edge", "Edge", 90)]);
    let crop = portrait_crop(edge.region("edge").unwrap(), edge.picture());
    assert_eq!(crop.left_permille, 0);
    assert!(crop.width_permille > 0);
}

#[test]
fn the_frame_of_a_crop_scales_the_picture_and_moves_it_to_the_cut() {
    let crop = PortraitCrop {
        left_permille: 500,
        top_permille: 250,
        width_permille: 250,
        height_permille: 500,
    };
    let (width, height, left, top) = crop.frame();
    assert_eq!((width, height), (400.0, 200.0));
    assert_eq!((left, top), (-200.0, -50.0));
}

#[test]
fn a_visible_speaker_has_a_hidden_lower_third_twin_and_an_off_screen_one_has_none() {
    let room = room();
    let bubbles = [bubble("innkeeper", BubbleTone::Npc, "Welcome.")];
    let twin = lower_third_twin(&bubbles, Some(&room), Some(&room)).expect("a twin");
    assert!(twin.twin);
    assert_eq!(twin.name, "Brann");
    assert_eq!(
        lower_third_twin(&bubbles, None, Some(&room)),
        None,
        "off screen, the speaker has the card, not a twin"
    );
    let thought = [bubble("innkeeper", BubbleTone::Thought, "")];
    assert_eq!(lower_third_twin(&thought, Some(&room), Some(&room)), None);
}

#[test]
fn the_portrait_holds_the_whole_face_with_room_above_the_head() {
    let room = room();
    let picture = room.picture();
    let region = room.region("innkeeper").expect("a region");
    let face = df_scene_layout::face_box(region, picture.width_px, picture.height_px);
    let crop = portrait_crop(region, picture);
    assert!(crop.top_permille < face.top_permille, "room above the head");
    assert!(crop.top_permille + crop.height_permille > face.bottom_permille);
    assert!(crop.left_permille < face.left_permille);
    assert!(crop.left_permille + crop.width_permille > face.right_permille);
}
