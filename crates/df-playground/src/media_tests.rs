use crate::fixtures::assert_keys_are_serde_names;
use crate::limits::{MAXIMUM_MEDIA_NAME_BYTES, MAXIMUM_REPORTED_ERRORS};
use crate::media::{
    AudioState, ClientReport, MediaKind, content_type, is_plain_file_name, media_kind,
};
use crate::timeline::{ClientError, ErrorSource, TimelineKind, TimelineRecord};

fn error(message: String) -> ClientError {
    ClientError {
        seq: 0,
        source: ErrorSource::Asset,
        page_ms: 0,
        message,
    }
}

#[test]
fn only_plain_names_pass() {
    for good in ["bell.wav", "a-b_c.1.png", "Tavern.PNG"] {
        assert!(is_plain_file_name(good), "{good}");
    }
    for bad in [
        "",
        ".x.png",
        "..",
        "a..b.png",
        "a/b.png",
        "a\\b.png",
        "a b.png",
        &"a".repeat(MAXIMUM_MEDIA_NAME_BYTES + 1),
    ] {
        assert!(!is_plain_file_name(bad), "{bad:?}");
    }
}

#[test]
fn a_name_is_an_image_or_a_sound_by_its_extension_without_regard_to_case() {
    assert_eq!(media_kind("tavern.png"), Some(MediaKind::Image));
    assert_eq!(media_kind("tavern.JPEG"), Some(MediaKind::Image));
    assert_eq!(media_kind("bell.wav"), Some(MediaKind::Audio));
    assert_eq!(media_kind("song.OGG"), Some(MediaKind::Audio));
}

#[test]
fn a_video_name_is_a_video_and_is_served_with_its_media_type() {
    assert_eq!(media_kind("hearth.mp4"), Some(MediaKind::Video));
    assert_eq!(media_kind("Rain.WEBM"), Some(MediaKind::Video));
    assert_eq!(content_type("a.mp4"), "video/mp4");
    assert_eq!(content_type("a.WEBM"), "video/webm");
    assert_eq!(MediaKind::Video.key(), "video");
    assert_eq!(media_kind("movie.mov"), None);
}

#[test]
fn svg_scripts_executables_and_names_without_an_extension_are_not_media() {
    for name in [
        "logo.svg",
        "run.exe",
        "page.html",
        "noextension",
        "notes.txt",
        "../a.png",
    ] {
        assert_eq!(media_kind(name), None, "{name}");
    }
}

#[test]
fn each_type_is_served_with_its_own_media_type() {
    assert_eq!(content_type("a.png"), "image/png");
    assert_eq!(content_type("a.JPG"), "image/jpeg");
    assert_eq!(content_type("a.webp"), "image/webp");
    assert_eq!(content_type("a.mp3"), "audio/mpeg");
    assert_eq!(content_type("a.wav"), "audio/wav");
    assert_eq!(content_type("a.bin"), "application/octet-stream");
    assert_eq!(MediaKind::Audio.key(), "audio");
}

fn report(errors: usize) -> ClientReport {
    ClientReport {
        errors: (0..errors)
            .map(|index| error(format!("error {index}")))
            .collect(),
        ..ClientReport::default()
    }
}

#[test]
fn a_report_keeps_only_the_newest_errors() {
    let bounded = report(MAXIMUM_REPORTED_ERRORS + 3).bounded();
    assert_eq!(bounded.errors.len(), MAXIMUM_REPORTED_ERRORS);
    assert_eq!(bounded.errors[0].message, "error 3");
    assert_eq!(
        bounded.errors.last().map(|error| error.message.as_str()),
        Some(format!("error {}", MAXIMUM_REPORTED_ERRORS + 2).as_str())
    );
    assert_eq!(report(2).bounded().errors.len(), 2);
}

#[test]
fn a_report_survives_a_round_trip_and_names_the_audio_state() {
    let original = ClientReport {
        audio: AudioState::Unlocked,
        last_cue: 4,
        music: Some("ambience.wav".into()),
        images_shown: vec!["tavern.png".into()],
        frame: Some(7),
        boot: Some("boot-1".into()),
        asset_revision: "00ff.00aa".into(),
        page_clock_ms: 1_234,
        errors: vec![error("a picture did not load".into())],
        timeline: vec![TimelineRecord {
            seq: 2,
            kind: TimelineKind::Line,
            id: "line-0000abcd".into(),
            start_page_ms: 10,
            end_page_ms: None,
            end_reason: None,
            planned_ms: Some(1_800),
            detail: None,
        }],
        ..ClientReport::default()
    };
    let text = serde_json::to_string(&original).expect("writes");
    assert!(text.contains(r#""audio":"unlocked""#));
    assert_eq!(
        serde_json::from_str::<ClientReport>(&text).expect("reads"),
        original
    );
}

#[test]
fn a_reports_texts_are_cut_to_their_length_and_its_pictures_to_their_count() {
    let long = "e".repeat(crate::limits::MAXIMUM_REPORTED_ERROR_CHARACTERS + 50);
    let report = ClientReport {
        music: Some("m".repeat(MAXIMUM_MEDIA_NAME_BYTES + 10)),
        images_shown: (0..crate::limits::MAXIMUM_REPORTED_IMAGES + 5)
            .map(|index| format!("{index}.png"))
            .collect(),
        errors: vec![error(long)],
        ..ClientReport::default()
    }
    .bounded();
    assert_eq!(
        report.errors[0].message.chars().count(),
        crate::limits::MAXIMUM_REPORTED_ERROR_CHARACTERS
    );
    assert_eq!(
        report.music.map(|music| music.chars().count()),
        Some(MAXIMUM_MEDIA_NAME_BYTES)
    );
    assert_eq!(
        report.images_shown.len(),
        crate::limits::MAXIMUM_REPORTED_IMAGES
    );
}

#[test]
fn a_report_is_held_to_its_total_size_dropping_the_oldest_errors_first() {
    let big = "x".repeat(crate::limits::MAXIMUM_REPORTED_ERROR_CHARACTERS);
    let errors: Vec<ClientError> = (0..crate::limits::MAXIMUM_REPORTED_ERRORS)
        .map(|index| error(format!("{index:04}{big}")))
        .collect();
    let newest = errors.last().map(|error| error.message[..4].to_owned());
    let report = ClientReport {
        images_shown: vec!["a.png".into()],
        errors,
        ..ClientReport::default()
    }
    .bounded();
    let held: usize = report
        .errors
        .iter()
        .map(|error| error.message.len())
        .sum::<usize>()
        + report.images_shown.iter().map(String::len).sum::<usize>();
    assert!(held <= crate::limits::MAXIMUM_REPORT_BYTES, "{held}");
    assert!(!report.errors.is_empty());
    assert_eq!(
        report
            .errors
            .last()
            .map(|error| error.message[..4].to_owned()),
        newest
    );
    assert_eq!(
        report.images_shown,
        ["a.png"],
        "pictures go only when the errors are gone"
    );
}

#[test]
fn a_multibyte_text_is_cut_on_a_character_and_never_panics() {
    let report = ClientReport {
        music: Some("\u{e9}".repeat(MAXIMUM_MEDIA_NAME_BYTES + 1)),
        ..ClientReport::default()
    }
    .bounded();
    assert_eq!(
        report.music.map(|music| music.chars().count()),
        Some(MAXIMUM_MEDIA_NAME_BYTES)
    );
}

#[test]
fn the_keys_of_a_media_kind_are_its_serde_names() {
    assert_keys_are_serde_names(&MediaKind::ALL, MediaKind::key, MediaKind::from_key);
}

#[test]
fn the_keys_of_an_audio_state_are_its_serde_names() {
    assert_keys_are_serde_names(&AudioState::ALL, AudioState::key, AudioState::from_key);
}

#[test]
fn a_report_with_a_misspelt_key_is_refused() {
    let report = serde_json::to_string(&ClientReport::default()).expect("writes");
    assert!(serde_json::from_str::<ClientReport>(&report).is_ok());
    let misspelt = report.replace("\"errors\":[]", "\"errors\":[],\"image_shown\":[]");
    assert_ne!(misspelt, report);
    assert!(serde_json::from_str::<ClientReport>(&misspelt).is_err());
}

#[test]
fn a_reports_timeline_is_held_to_its_count_keeping_the_newest() {
    let records = (0..crate::limits::MAXIMUM_TIMELINE_RECORDS as u64 + 5)
        .map(|seq| TimelineRecord {
            seq,
            kind: TimelineKind::Figure,
            id: "x".into(),
            start_page_ms: seq,
            end_page_ms: None,
            end_reason: None,
            planned_ms: None,
            detail: None,
        })
        .collect();
    let report = ClientReport {
        timeline: records,
        ..ClientReport::default()
    }
    .bounded();
    assert_eq!(
        report.timeline.len(),
        crate::limits::MAXIMUM_TIMELINE_RECORDS
    );
    assert_eq!(report.timeline[0].seq, 5, "the oldest go first");
}

#[test]
fn a_reports_timeline_ids_are_cut_to_their_length() {
    let report = ClientReport {
        timeline: vec![TimelineRecord {
            seq: 1,
            kind: TimelineKind::Figure,
            id: "x".repeat(crate::limits::MAXIMUM_TIMELINE_ID_CHARACTERS + 3),
            start_page_ms: 1,
            end_page_ms: None,
            end_reason: None,
            planned_ms: None,
            detail: None,
        }],
        ..ClientReport::default()
    }
    .bounded();
    assert_eq!(
        report.timeline[0].id.chars().count(),
        crate::limits::MAXIMUM_TIMELINE_ID_CHARACTERS
    );
}

#[test]
fn a_reports_timeline_is_held_to_the_report_bytes_keeping_the_newest() {
    let id_characters = crate::limits::MAXIMUM_TIMELINE_ID_CHARACTERS;
    let count = crate::limits::MAXIMUM_REPORT_BYTES / id_characters + 10;
    let records = (0..count as u64)
        .map(|seq| TimelineRecord {
            seq,
            kind: TimelineKind::Figure,
            id: "x".repeat(id_characters),
            start_page_ms: seq,
            end_page_ms: None,
            end_reason: None,
            planned_ms: None,
            detail: None,
        })
        .collect();
    let report = ClientReport {
        timeline: records,
        ..ClientReport::default()
    }
    .bounded();
    // The count bound and the byte bound both hold; the smaller one is what is kept.
    let kept = (crate::limits::MAXIMUM_REPORT_BYTES / id_characters)
        .min(crate::limits::MAXIMUM_TIMELINE_RECORDS);
    assert_eq!(report.timeline.len(), kept);
    assert_eq!(
        report.timeline.last().map(|record| record.seq),
        Some(count as u64 - 1)
    );
}
