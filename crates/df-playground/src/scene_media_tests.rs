//! Pictures, music and sound cues in the scene overlay.

use crate::controls::Device;
use crate::fixture::{Fixture, ImageRef};
use crate::scene::{
    Cue, ImageSlot, MAXIMUM_CUES_KEPT, Override, SceneChange, SceneError, SceneOverlay, Target,
    Track,
};

const FIXTURE: &str = r#"{
    "name": "Base",
    "tv": {
        "place": "Here", "status": {"kind": "ok"}, "elements": [],
        "backdrop": {"media": "tavern.png", "caption": "The Gilded Flagon"}
    },
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

fn base() -> Fixture {
    Fixture::parse(FIXTURE).expect("the fixture reads")
}

fn drawn(overlay: &SceneOverlay) -> Fixture {
    let mut fixture = base();
    overlay.apply_to(&mut fixture);
    fixture
}

fn image(media: &str) -> ImageRef {
    ImageRef {
        media: media.to_owned(),
        caption: None,
    }
}

fn sound(media: &str, volume: u8) -> SceneChange {
    SceneChange::Sound {
        media: media.to_owned(),
        target: Target::Both,
        volume,
    }
}

#[test]
fn a_fixture_may_carry_pictures_and_refuses_unknown_picture_keys() {
    let fixture = base();
    assert_eq!(
        fixture.tv.backdrop,
        Some(ImageRef {
            media: "tavern.png".into(),
            caption: Some("The Gilded Flagon".into())
        })
    );
    assert_eq!((fixture.tv.card, fixture.phone.image), (None, None));
    let bad = FIXTURE.replace("\"media\"", "\"picture\"");
    assert!(Fixture::parse(&bad).is_err());
}

#[test]
fn a_picture_is_set_in_its_slot_and_replaces_what_the_fixture_has() {
    let mut overlay = SceneOverlay::default();
    for (slot, media) in [
        (ImageSlot::TvBackdrop, "cellar.png"),
        (ImageSlot::TvCard, "letter.jpg"),
        (ImageSlot::PhoneImage, "map.webp"),
    ] {
        overlay
            .apply(
                SceneChange::Image {
                    slot,
                    media: media.into(),
                    caption: None,
                },
                &base(),
            )
            .expect("applies");
    }
    let fixture = drawn(&overlay);
    assert_eq!(fixture.tv.backdrop, Some(image("cellar.png")));
    assert_eq!(fixture.tv.card, Some(image("letter.jpg")));
    assert_eq!(fixture.phone.image, Some(image("map.webp")));
}

#[test]
fn clearing_a_slot_removes_the_fixtures_picture_and_other_slots_are_left_alone() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(
            SceneChange::ClearImage {
                slot: ImageSlot::TvBackdrop,
            },
            &base(),
        )
        .expect("applies");
    assert_eq!(overlay.backdrop, Some(Override::Clear));
    assert_eq!(drawn(&overlay).tv.backdrop, None);
    assert!(overlay.card.is_none() && overlay.phone_image.is_none());
}

#[test]
fn a_picture_needs_a_name_and_a_caption_within_the_limit() {
    let mut overlay = SceneOverlay::default();
    let empty = SceneChange::Image {
        slot: ImageSlot::TvCard,
        media: String::new(),
        caption: None,
    };
    assert_eq!(
        overlay.apply(empty, &base()),
        Err(SceneError::Empty("a media name"))
    );
    let long = SceneChange::Image {
        slot: ImageSlot::TvCard,
        media: "a.png".into(),
        caption: Some("x".repeat(200)),
    };
    assert!(matches!(
        overlay.apply(long, &base()),
        Err(SceneError::TooLong { .. })
    ));
    assert!(overlay.is_empty());
}

#[test]
fn music_is_set_replaced_and_stopped_and_never_changes_the_fixture() {
    let mut overlay = SceneOverlay::default();
    let music = |media: &str| SceneChange::Music {
        media: media.into(),
        target: Target::Tv,
        volume: 40,
        looping: true,
    };
    overlay.apply(music("a.wav"), &base()).expect("applies");
    overlay.apply(music("b.wav"), &base()).expect("replaces");
    assert_eq!(
        overlay.music,
        Some(Override::Set(Track {
            media: "b.wav".into(),
            target: Target::Tv,
            volume: 40,
            looping: true
        }))
    );
    assert_eq!(drawn(&overlay), base());
    overlay
        .apply(SceneChange::StopMusic {}, &base())
        .expect("stops");
    assert_eq!(overlay.music, Some(Override::Clear));
}

#[test]
fn a_volume_over_a_hundred_percent_is_refused_for_music_and_for_a_sound() {
    let mut overlay = SceneOverlay::default();
    assert_eq!(
        overlay.apply(
            SceneChange::Music {
                media: "a.wav".into(),
                target: Target::Both,
                volume: 101,
                looping: false
            },
            &base()
        ),
        Err(SceneError::BadVolume(101))
    );
    assert_eq!(
        overlay.apply(sound("a.wav", 255), &base()),
        Err(SceneError::BadVolume(255))
    );
    assert!(overlay.apply(sound("a.wav", 100), &base()).is_ok());
    assert!(SceneError::BadVolume(101).to_string().contains("0 to 100"));
}

#[test]
fn each_sound_is_a_numbered_cue_and_only_the_newest_are_kept() {
    let mut overlay = SceneOverlay::default();
    for _ in 0..MAXIMUM_CUES_KEPT + 4 {
        overlay
            .apply(sound("bell.wav", 80), &base())
            .expect("applies");
    }
    assert_eq!(overlay.cue_count, (MAXIMUM_CUES_KEPT + 4) as u64);
    assert_eq!(overlay.cues.len(), MAXIMUM_CUES_KEPT);
    assert_eq!(overlay.cues[0].id, 5, "the oldest four were dropped");
    assert_eq!(
        overlay.cues.last(),
        Some(&Cue {
            id: (MAXIMUM_CUES_KEPT + 4) as u64,
            target: Target::Both,
            media: "bell.wav".into(),
            volume: 80
        })
    );
}

#[test]
fn a_reset_drops_the_changes_and_the_old_cues_but_the_count_keeps_rising() {
    let mut overlay = SceneOverlay::default();
    overlay.apply(sound("a.wav", 50), &base()).expect("applies");
    overlay.apply(sound("a.wav", 50), &base()).expect("applies");
    overlay
        .apply(
            SceneChange::Image {
                slot: ImageSlot::TvCard,
                media: "x.png".into(),
                caption: None,
            },
            &base(),
        )
        .expect("applies");
    overlay
        .apply(SceneChange::Reset {}, &base())
        .expect("resets");
    assert!(overlay.is_empty() && overlay.cues.is_empty());
    assert_eq!(overlay.cue_count, 2);
    overlay.apply(sound("a.wav", 50), &base()).expect("applies");
    assert_eq!(
        overlay.cues[0].id, 3,
        "a page that has played 2 still sees this one as new"
    );
    assert_eq!(overlay.cleared().cue_count, 3);
}

#[test]
fn past_cues_alone_do_not_make_the_overlay_count_as_changed() {
    let mut overlay = SceneOverlay::default();
    overlay.apply(sound("a.wav", 50), &base()).expect("applies");
    assert!(overlay.is_empty(), "nothing about what is drawn changed");
    overlay
        .apply(SceneChange::StopMusic {}, &base())
        .expect("applies");
    assert!(!overlay.is_empty());
}

#[test]
fn a_target_decides_which_screens_play_a_sound() {
    assert!(Target::Both.plays_on(Device::Tv) && Target::Both.plays_on(Device::Phone));
    assert!(Target::Tv.plays_on(Device::Tv) && !Target::Tv.plays_on(Device::Phone));
    assert!(Target::Tv.plays_on(Device::Both));
    assert!(Target::Phone.plays_on(Device::Phone) && !Target::Phone.plays_on(Device::Tv));
    assert_eq!(Target::from_key("tv"), Some(Target::Tv));
    assert_eq!(Target::from_key("watch"), None);
    assert_eq!(
        ImageSlot::from_key("tv-backdrop"),
        Some(ImageSlot::TvBackdrop)
    );
    assert_eq!(ImageSlot::from_key("phone"), Some(ImageSlot::PhoneImage));
    assert_eq!(ImageSlot::from_key("wall"), None);
}

#[test]
fn media_changes_are_written_and_read_by_name() {
    let change = SceneChange::Music {
        media: "a.wav".into(),
        target: Target::Phone,
        volume: 30,
        looping: false,
    };
    let text = serde_json::to_string(&change).expect("writes");
    assert_eq!(
        text,
        r#"{"change":"music","media":"a.wav","target":"phone","volume":30,"looping":false}"#
    );
    assert_eq!(
        serde_json::from_str::<SceneChange>(&text).expect("reads"),
        change
    );
    let slot = r#"{"change":"clear_image","slot":"tv_backdrop"}"#;
    assert_eq!(
        serde_json::from_str::<SceneChange>(slot).expect("reads"),
        SceneChange::ClearImage {
            slot: ImageSlot::TvBackdrop
        }
    );
}
