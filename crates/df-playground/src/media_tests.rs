use crate::media::{
    AudioState, ClientReport, MAXIMUM_REPORTED_ERRORS, MediaKind, content_type, is_plain_file_name,
    media_kind,
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
        &"a".repeat(65),
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
    assert_eq!(bounded.errors.last().map(String::as_str), Some("error 10"));
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
