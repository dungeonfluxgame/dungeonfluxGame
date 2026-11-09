//! The close-up decision, the title and description split, and how a live caption change carries
//! its title.

use df_scene_layout::{RegionGeometry, RegionId};

use crate::closeup::{CloseUp, scene_closeup};
use crate::fixture::{Fixture, ImageRef, PlateKind};
use crate::image_layout::{ImageLayout, LayoutRegion, PictureBinding};
use crate::lobby::change::{RoleplayEdit, ScreenEdit};
use crate::scene::{SceneChange, SceneOverlay};

const FIXTURE: &str = r#"{
    "name": "Base",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": []},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

fn layout_of(picture: &str) -> ImageLayout {
    let geometry = RegionGeometry {
        left_permille: 220,
        top_permille: 200,
        right_permille: 380,
        bottom_permille: 1000,
        head_x_permille: 300,
        head_y_permille: 220,
    };
    let region = LayoutRegion::new(RegionId::new("r00000001").unwrap(), geometry, 950).unwrap();
    let binding = PictureBinding {
        name: picture.to_owned(),
        sha256: "ab".repeat(32),
        width_px: 1600,
        height_px: 900,
    };
    ImageLayout::new(binding, "sam-3.1", "person", vec![region])
}

fn backdrop(media: &str, layout: Option<ImageLayout>, plate: PlateKind) -> ImageRef {
    ImageRef {
        media: media.to_owned(),
        caption: None,
        rotation_degrees: 0,
        slide: None,
        layout,
        plate,
    }
}

/// A scene whose backdrop is `media` carrying `layout`, put up as `plate`, with the caption and
/// title sent.
fn scene(
    media: &str,
    layout: Option<ImageLayout>,
    plate: PlateKind,
    edit: RoleplayEdit,
) -> Fixture {
    let mut fixture = Fixture::parse(FIXTURE).expect("the fixture reads");
    fixture.tv.backdrop = Some(backdrop(media, layout, plate));
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(
            SceneChange::Screen {
                edit: ScreenEdit::RoleplaySet { roleplay: edit },
            },
            &fixture,
        )
        .expect("the change applies");
    overlay.apply_to(&mut fixture);
    fixture
}

fn caption(text: &str, title: Option<&str>) -> RoleplayEdit {
    RoleplayEdit {
        caption: Some(text.to_owned()),
        caption_title: title.map(str::to_owned),
        ..RoleplayEdit::default()
    }
}

#[test]
fn a_backdrop_the_host_put_up_as_a_close_up_is_one() {
    let fixture = scene(
        "latch.webp",
        Some(layout_of("room.webp")),
        PlateKind::CloseUp,
        caption("Scratched.", None),
    );
    assert!(fixture.tv.is_close_up());
    assert!(fixture.tv.backdrop_layout().is_none(), "nobody is outlined");
    assert!(fixture.tv.cast_layout().is_some(), "the cast may speak");
}

#[test]
fn a_room_plate_is_never_a_close_up_whatever_its_layout_was_scanned_from() {
    let room = scene(
        "room.webp",
        Some(layout_of("room.webp")),
        PlateKind::Room,
        caption("The party arrives.", Some("Room")),
    );
    assert!(!room.tv.is_close_up());
    assert_eq!(scene_closeup(&room.tv), None);
    let kept = scene(
        "room-2.webp",
        Some(layout_of("room.webp")),
        PlateKind::RoomKeptCast,
        caption("The party arrives.", Some("Room")),
    );
    assert!(!kept.tv.is_close_up());
    assert_eq!(scene_closeup(&kept.tv), None);
    assert!(kept.tv.backdrop_layout().is_none(), "nobody is outlined");
    assert!(kept.tv.cast_layout().is_some(), "the cast may speak");
    let bare = scene(
        "latch.webp",
        None,
        PlateKind::Room,
        caption("Scratched.", Some("Latch")),
    );
    assert!(!bare.tv.is_close_up());
    assert_eq!(scene_closeup(&bare.tv), None);
}

#[test]
fn a_close_up_caption_splits_into_the_items_name_and_its_words() {
    let fixture = scene(
        "latch.webp",
        Some(layout_of("room.webp")),
        PlateKind::CloseUp,
        caption("  Fresh scratches ring the latch.  ", Some(" Brass latch ")),
    );
    assert_eq!(
        scene_closeup(&fixture.tv),
        Some(CloseUp {
            title: Some("Brass latch".to_owned()),
            description: "Fresh scratches ring the latch.".to_owned(),
        })
    );
}

#[test]
fn without_a_name_the_words_stand_alone() {
    for title in [None, Some("  ")] {
        let fixture = scene(
            "latch.webp",
            Some(layout_of("room.webp")),
            PlateKind::CloseUp,
            caption("Fresh scratches.", title),
        );
        assert_eq!(
            scene_closeup(&fixture.tv),
            Some(CloseUp {
                title: None,
                description: "Fresh scratches.".to_owned(),
            })
        );
    }
}

#[test]
fn a_close_up_without_words_has_no_card() {
    let fixture = scene(
        "latch.webp",
        Some(layout_of("room.webp")),
        PlateKind::CloseUp,
        caption("", Some("Brass latch")),
    );
    assert_eq!(scene_closeup(&fixture.tv), None);
}

#[test]
fn a_caption_change_replaces_the_title_and_taking_the_caption_away_takes_the_title() {
    let mut fixture = scene(
        "latch.webp",
        Some(layout_of("room.webp")),
        PlateKind::CloseUp,
        caption("Scratched.", Some("Latch")),
    );
    let mut overlay = SceneOverlay::default();
    for edit in [caption("Scratched.", None), caption("", None)] {
        overlay
            .apply(
                SceneChange::Screen {
                    edit: ScreenEdit::RoleplaySet { roleplay: edit },
                },
                &fixture,
            )
            .expect("the change applies");
        overlay.apply_to(&mut fixture);
    }
    let card = fixture.tv.roleplay.expect("a card");
    assert_eq!((card.caption, card.caption_title), (None, None));
}

#[test]
fn the_cards_identity_changes_with_either_part() {
    let card = CloseUp {
        title: Some("Latch".to_owned()),
        description: "Scratched.".to_owned(),
    };
    let renamed = CloseUp {
        title: Some("Lock".to_owned()),
        ..card.clone()
    };
    let bare = CloseUp {
        title: None,
        ..card.clone()
    };
    assert_ne!(card.card_identity(), renamed.card_identity());
    assert_ne!(card.card_identity(), bare.card_identity());
}
