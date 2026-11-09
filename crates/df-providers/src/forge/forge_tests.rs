use super::{
    FORGE_TTS_SPEAKERS, ForgeCause, ForgeConfig, MAXIMUM_REASON_CHARACTERS, forge_speaker,
    picture_type, refusal_reason,
};

const PROJECT: &str = "41fbb3c9-ac91-4897-b890-7a056b4ae371";

#[test]
fn only_a_plain_loopback_origin_and_a_project_uuid_configure_the_client() {
    for origin in [
        "http://127.0.0.1:5173",
        "http://127.0.0.1:5173/",
        "http://localhost:5173",
        "http://[::1]:5173",
    ] {
        let config = ForgeConfig::new(origin, PROJECT).unwrap();
        assert_eq!(config.origin(), origin.trim_end_matches('/'));
    }
    for origin in [
        "https://127.0.0.1:5173",
        "http://192.168.1.4:5173",
        "http://forge.example:5173",
        "http://127.0.0.1",
        "http://127.0.0.1:0",
        "http://127.0.0.1:5173/api",
        "http://user@127.0.0.1:5173",
    ] {
        let refused = ForgeConfig::new(origin, PROJECT).unwrap_err();
        assert_eq!(refused.cause, ForgeCause::InvalidConfiguration, "{origin}");
    }
    for project in [
        "",
        "DungeonFlux dev",
        "41fbb3c9ac914897b8907a056b4ae371",
        "../x",
    ] {
        assert!(ForgeConfig::new("http://127.0.0.1:5173", project).is_err());
    }
}

#[test]
fn a_preset_speaker_is_kept_and_any_other_voice_maps_to_one_stably() {
    assert_eq!(forge_speaker("Aiden"), "Aiden");
    assert_eq!(forge_speaker("ono_anna"), "Ono_Anna");
    let mapped = forge_speaker("JBFqnCBsd6RMkjVDRZzb");
    assert!(FORGE_TTS_SPEAKERS.contains(&mapped));
    assert_eq!(forge_speaker("JBFqnCBsd6RMkjVDRZzb"), mapped);
    // Different voice ids spread over the speakers rather than all landing on one.
    let spread: std::collections::BTreeSet<&str> = (0..64)
        .map(|n| forge_speaker(&format!("voice-{n}")))
        .collect();
    assert!(spread.len() > 4, "{spread:?}");
}

#[test]
fn a_staged_picture_is_typed_by_its_signature() {
    assert_eq!(picture_type(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
    assert_eq!(picture_type(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
    assert_eq!(picture_type(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
    assert_eq!(picture_type(b"GIF89a"), None);
    assert_eq!(picture_type(b""), None);
}

#[test]
fn a_refusal_keeps_forge_s_sentence_bounded_and_on_one_line() {
    let refusal =
        r#"{"error": "Image description must contain 1–4000 characters.", "supervisor": {}}"#;
    assert_eq!(
        refusal_reason(refusal.as_bytes()).as_deref(),
        Some("Image description must contain 1–4000 characters.")
    );
    assert_eq!(
        refusal_reason(br#"{"error": "first\nsecond"}"#).as_deref(),
        Some("first second")
    );
    let long = format!(
        r#"{{"error": "{}"}}"#,
        "x".repeat(MAXIMUM_REASON_CHARACTERS + 1)
    );
    assert_eq!(
        refusal_reason(long.as_bytes()).map(|reason| reason.chars().count()),
        Some(MAXIMUM_REASON_CHARACTERS)
    );
    for body in [
        &b"not json"[..],
        br#"{"error": ""}"#,
        br#"{"error": 3}"#,
        b"{}",
    ] {
        assert_eq!(refusal_reason(body), None);
    }
}
