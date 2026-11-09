use super::{FORGE_TTS_SPEAKERS, ForgeCause, ForgeConfig, forge_speaker, picture_type};

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
