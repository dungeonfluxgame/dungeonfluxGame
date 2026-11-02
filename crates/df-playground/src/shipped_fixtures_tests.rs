//! The fixtures shipped in `assets/playground/fixtures` must read, and all but the one that exists
//! to break the rules must satisfy the choice-list contract.

use std::path::PathBuf;

use crate::choices::build;
use crate::fixture::Fixture;

/// The fixture that breaks the contract on purpose, so the warnings have something to show.
const DELIBERATELY_BROKEN: &str = "contract_breaks.json";

fn fixtures() -> Vec<(String, Fixture)> {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/playground/fixtures");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&directory).expect("the fixture directory exists") {
        let path = entry.expect("reads an entry").path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let text = std::fs::read_to_string(&path).expect("reads the fixture");
            let name = path
                .file_name()
                .expect("has a name")
                .to_string_lossy()
                .into_owned();
            let fixture = Fixture::parse(&text).unwrap_or_else(|error| {
                panic!(
                    "{name}, line {}, column {}: {}",
                    error.line, error.column, error.message
                )
            });
            found.push((name, fixture));
        }
    }
    found.sort_by(|left, right| left.0.cmp(&right.0));
    found
}

#[test]
fn every_shipped_fixture_reads() {
    assert!(fixtures().len() >= 4);
}

#[test]
fn every_shipped_fixture_but_the_deliberate_one_follows_the_choice_rules() {
    for (name, fixture) in fixtures() {
        let warnings = build(&fixture.phone, true).warnings;
        if name == DELIBERATELY_BROKEN {
            assert!(!warnings.is_empty(), "{name} should break the rules");
        } else {
            assert!(warnings.is_empty(), "{name}: {warnings:?}");
        }
    }
}

#[test]
fn every_picture_a_shipped_fixture_names_is_in_the_media_library_and_is_a_picture() {
    use crate::media::{MediaKind, media_kind};
    let media = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/playground/media");
    for (name, fixture) in fixtures() {
        let slots = [
            fixture.tv.backdrop.as_ref(),
            fixture.tv.card.as_ref(),
            fixture.phone.image.as_ref(),
        ];
        for image in slots.into_iter().flatten() {
            assert_eq!(
                media_kind(&image.media),
                Some(MediaKind::Image),
                "{name}: {}",
                image.media
            );
            assert!(
                media.join(&image.media).is_file(),
                "{name} names {}, which is not in assets/playground/media",
                image.media
            );
        }
    }
}

#[test]
fn the_shipped_media_are_plain_names_of_known_types_within_the_size_limit() {
    use crate::media::media_kind;
    let media = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/playground/media");
    let mut count = 0;
    for entry in std::fs::read_dir(&media).expect("the media directory exists") {
        let entry = entry.expect("reads an entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            media_kind(&name).is_some(),
            "{name} is not a known media file"
        );
        assert!(entry.metadata().expect("has metadata").len() < 16 * 1024 * 1024);
        count += 1;
    }
    assert!(count >= 5);
}

#[test]
fn every_music_a_shipped_fixture_plays_is_a_sound_in_the_media_library() {
    use crate::media::{MediaKind, media_kind};
    let media = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/playground/media");
    let mut with_music = 0;
    for (name, fixture) in fixtures() {
        if let Some(track) = &fixture.music {
            with_music += 1;
            assert_eq!(media_kind(&track.media), Some(MediaKind::Audio), "{name}");
            assert!(
                media.join(&track.media).is_file(),
                "{name} plays {}",
                track.media
            );
            assert!(track.volume <= 100, "{name}");
        }
    }
    assert!(with_music >= 2, "the two scenes each bring their own music");
}

#[test]
fn the_two_scenes_to_move_between_exist_and_differ_in_picture_music_and_place() {
    let all = fixtures();
    let find = |wanted: &str| {
        all.iter()
            .find(|(name, _)| name == wanted)
            .map(|(_, fixture)| fixture.clone())
            .unwrap_or_else(|| panic!("{wanted} is missing"))
    };
    let (quay, tavern) = (find("scene_quay.json"), find("scene_tavern.json"));
    assert_ne!(quay.tv.place, tavern.tv.place);
    assert_ne!(quay.tv.backdrop, tavern.tv.backdrop);
    assert_ne!(quay.music, tavern.music);
    assert_ne!(quay.phone.hero, tavern.phone.hero);
}
