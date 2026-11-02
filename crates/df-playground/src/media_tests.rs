use crate::media::{
    AudioState, ClientReport, MAXIMUM_NAME_LENGTH, MAXIMUM_REPORTED_ERRORS, MediaKind,
    content_type, is_plain_file_name, media_kind,
};

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
        &"a".repeat(MAXIMUM_NAME_LENGTH + 1),
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
        audio: AudioState::Locked,
        last_cue: 0,
        music: None,
        images_shown: Vec::new(),
        errors: (0..errors).map(|index| format!("error {index}")).collect(),
    }
}

#[test]
fn a_report_keeps_only_the_newest_errors() {
    let bounded = report(MAXIMUM_REPORTED_ERRORS + 3).bounded();
    assert_eq!(bounded.errors.len(), MAXIMUM_REPORTED_ERRORS);
    assert_eq!(bounded.errors[0], "error 3");
    assert_eq!(
        bounded.errors.last().map(String::as_str),
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
        errors: Vec::new(),
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
        audio: AudioState::Locked,
        last_cue: 0,
        music: Some("m".repeat(MAXIMUM_NAME_LENGTH + 10)),
        images_shown: (0..crate::limits::MAXIMUM_REPORTED_IMAGES + 5)
            .map(|index| format!("{index}.png"))
            .collect(),
        errors: vec![long],
    }
    .bounded();
    assert_eq!(
        report.errors[0].chars().count(),
        crate::limits::MAXIMUM_REPORTED_ERROR_CHARACTERS
    );
    assert_eq!(
        report.music.map(|music| music.chars().count()),
        Some(MAXIMUM_NAME_LENGTH)
    );
    assert_eq!(
        report.images_shown.len(),
        crate::limits::MAXIMUM_REPORTED_IMAGES
    );
}

#[test]
fn a_report_is_held_to_its_total_size_dropping_the_oldest_errors_first() {
    let big = "x".repeat(crate::limits::MAXIMUM_REPORTED_ERROR_CHARACTERS);
    let errors: Vec<String> = (0..crate::limits::MAXIMUM_REPORTED_ERRORS)
        .map(|index| format!("{index:04}{big}"))
        .collect();
    let newest = errors.last().cloned().map(|text| text[..4].to_owned());
    let report = ClientReport {
        audio: AudioState::Locked,
        last_cue: 0,
        music: None,
        images_shown: vec!["a.png".into()],
        errors,
    }
    .bounded();
    let held: usize = report.errors.iter().map(String::len).sum::<usize>()
        + report.images_shown.iter().map(String::len).sum::<usize>();
    assert!(held <= crate::limits::MAXIMUM_REPORT_BYTES, "{held}");
    assert!(!report.errors.is_empty());
    assert_eq!(
        report.errors.last().map(|text| text[..4].to_owned()),
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
        audio: AudioState::Locked,
        last_cue: 0,
        music: Some("\u{e9}".repeat(MAXIMUM_NAME_LENGTH + 1)),
        images_shown: Vec::new(),
        errors: Vec::new(),
    }
    .bounded();
    assert_eq!(
        report.music.map(|music| music.chars().count()),
        Some(MAXIMUM_NAME_LENGTH)
    );
}
