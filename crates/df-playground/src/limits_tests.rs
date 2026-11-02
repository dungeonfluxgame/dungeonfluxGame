//! The owner's rule: no invented small caps. Every limit is a named constant in `limits`; a value a
//! person could plausibly use is accepted, and the rules of the domain are kept.

use crate::controls::{Controls, ControlsChange};
use crate::fixture::{Choice, Fixture, PartyMember};
use crate::limits;
use crate::media::is_plain_file_name;
use crate::scene::{ImageSlot, SceneChange, SceneError, SceneOverlay, Target};

fn base() -> Fixture {
    Fixture::parse(
        r#"{"name":"B","tv":{"place":"P","status":{"kind":"ok"},"elements":[]},"phone":{"hero":"H","status":{"kind":"ok"}}}"#,
    )
    .expect("reads")
}

#[test]
fn the_domain_rules_are_kept() {
    assert_eq!(limits::MAXIMUM_ROTATION_DEGREES, 359);
    assert_eq!(limits::MINIMUM_ZOOM_PERCENT, 100);
    assert_eq!(limits::MAXIMUM_VOLUME, 100);
    assert_eq!(limits::MAXIMUM_SLIDE_PERCENT, 100);
    assert_eq!(limits::MAXIMUM_PIVOT_PERCENT, 100);
}

#[test]
fn every_invented_cap_is_very_large() {
    const {
        assert!(limits::MAXIMUM_CAPTION_CHARACTERS >= 100_000);
        assert!(limits::MAXIMUM_NAME_CHARACTERS >= 1_000);
        assert!(limits::MAXIMUM_PARTY_MEMBERS >= 1_000);
        assert!(limits::MAXIMUM_CHOICES_PER_LIST >= 1_000);
        assert!(limits::MAXIMUM_CUES_KEPT >= 4_096);
        assert!(limits::MAXIMUM_SLIDE_MS >= 86_400_000);
        assert!(limits::MAXIMUM_CAMERA_MS >= 86_400_000);
        assert!(limits::MAXIMUM_TRANSITION_MS >= 86_400_000);
        assert!(limits::MAXIMUM_ZOOM_PERCENT >= 10_000);
        assert!(limits::MAXIMUM_REMEMBERED_SLIDES >= 1_024);
        assert!(limits::MAXIMUM_REPORTED_ERRORS >= 1_000);
        assert!(limits::MAXIMUM_MEDIA_NAME_LENGTH >= 255);
    }
}

#[test]
fn the_other_modules_say_the_same_numbers() {
    assert_eq!(
        crate::scene::MAXIMUM_CAPTION_CHARACTERS,
        limits::MAXIMUM_CAPTION_CHARACTERS
    );
    assert_eq!(crate::scene::MAXIMUM_CUES_KEPT, limits::MAXIMUM_CUES_KEPT);
    assert_eq!(
        crate::controls::MAXIMUM_TRANSITION_MS,
        limits::MAXIMUM_TRANSITION_MS
    );
    assert_eq!(
        crate::media::MAXIMUM_NAME_LENGTH,
        limits::MAXIMUM_MEDIA_NAME_LENGTH
    );
    assert_eq!(
        crate::media::MAXIMUM_REPORTED_ERRORS,
        limits::MAXIMUM_REPORTED_ERRORS
    );
    assert_eq!(crate::backoff::LONGEST_RETRY_MS, limits::LONGEST_RETRY_MS);
}

#[test]
fn values_the_old_caps_refused_are_accepted() {
    let mut overlay = SceneOverlay::default();
    let fixture = base();
    overlay
        .apply(
            SceneChange::Caption {
                speaker: None,
                text: "x".repeat(601),
            },
            &fixture,
        )
        .expect("a caption of 601 characters");
    for index in 0..9 {
        overlay
            .apply(
                SceneChange::PartyAdd {
                    member: PartyMember {
                        name: format!("member {index}"),
                        hit_points: 1,
                        maximum_hit_points: 1,
                        conditions: Vec::new(),
                        color: None,
                    },
                },
                &fixture,
            )
            .expect("a ninth member");
    }
    let choices: Vec<Choice> = (0..25)
        .map(|index| Choice {
            label: format!("choice {index}"),
            explain: None,
        })
        .collect();
    overlay
        .apply(SceneChange::Suggested { choices }, &fixture)
        .expect("25 choices");
    for _ in 0..17 {
        overlay
            .apply(
                SceneChange::Sound {
                    media: "bell.wav".to_owned(),
                    target: Target::Both,
                    volume: 80,
                },
                &fixture,
            )
            .expect("a sound");
    }
    assert_eq!(overlay.cues.len(), 17, "17 cues are kept, not 16");
    overlay
        .apply(
            SceneChange::Camera {
                target: Some("a".to_owned()),
                zoom_percent: 401,
                duration_ms: 30_001,
            },
            &fixture,
        )
        .expect("a zoom of 401 percent over 30 001 ms");
    overlay
        .apply(
            SceneChange::Image {
                slot: ImageSlot::TvCard,
                media: "a.png".to_owned(),
                caption: None,
                rotation_degrees: 0,
                slide: Some(crate::fixture::Slide {
                    from_percent: 0,
                    to_percent: 100,
                    duration_ms: 60_001,
                    serial: 0,
                    began_ms: 0,
                    elapsed_ms: 0,
                }),
            },
            &fixture,
        )
        .expect("a slide of 60 001 ms");
    Controls::default()
        .apply(ControlsChange::TransitionMs(5_001))
        .expect("a transition of 5 001 ms");
    assert!(is_plain_file_name(&"a".repeat(65)));
}

#[test]
fn a_cap_still_refuses_what_is_past_it_and_the_words_name_the_constant() {
    let mut overlay = SceneOverlay::default();
    let too_long = SceneChange::Caption {
        speaker: None,
        text: "x".repeat(limits::MAXIMUM_CAPTION_CHARACTERS + 1),
    };
    assert_eq!(
        overlay.apply(too_long, &base()),
        Err(SceneError::TooLong {
            what: "the caption",
            limit: limits::MAXIMUM_CAPTION_CHARACTERS
        })
    );
    assert!(
        SceneError::TooManyMembers
            .to_string()
            .contains(&limits::MAXIMUM_PARTY_MEMBERS.to_string())
    );
    assert!(
        SceneError::BadCameraTime(1)
            .to_string()
            .contains(&limits::MAXIMUM_CAMERA_MS.to_string())
    );
}

#[test]
fn the_rotation_range_takes_zero_and_359_and_refuses_360() {
    let turn = |degrees: u16| SceneChange::Image {
        slot: ImageSlot::TvBackdrop,
        media: "a.png".to_owned(),
        caption: None,
        rotation_degrees: degrees,
        slide: None,
    };
    let mut overlay = SceneOverlay::default();
    assert!(overlay.apply(turn(0), &base()).is_ok());
    assert!(overlay.apply(turn(359), &base()).is_ok());
    assert_eq!(
        overlay.apply(turn(360), &base()),
        Err(SceneError::BadRotation(360))
    );
}
