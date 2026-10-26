//! The fake-provider profile: one switch (`DF_FAKE_PROVIDERS=on`) that lets a full table run
//! with no network provider. Every paid capability is switched off at its existing gate, and
//! the fixed harbor pack supplies deterministic text, and callers that need bytes get
//! silent audio and a placeholder picture from this module. It never contacts a provider and holds no secret.

use std::{
    env::VarError,
    io,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether startup explicitly installed the isolated fake-provider profile.
pub(crate) fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The switch that selects this profile. Exactly `on` selects it; unset, empty or `off` do not.
const PROFILE_VARIABLE: &str = "DF_FAKE_PROVIDERS";
const PLACEHOLDER_PATH: &str = "/assets/ui/scenes/mara-harbor-v4.png";
/// An accepted live-image reference with deterministic prepared PNG bytes, fake profile only.
pub(crate) const PORTRAIT_PATH: &str =
    "/gameplay/imagery/6733679ea2e93f6e76e84a7360c718313d9a7663442011c69523cee0b0d2afcb";

/// The selected fake-provider profile. Its only state is that it was selected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FakeProfile;

/// Parses the profile switch. An unknown value is refused without echoing it.
fn parse_profile(input: Result<String, VarError>) -> Result<Option<FakeProfile>, io::Error> {
    match input.as_deref() {
        Err(VarError::NotPresent) | Ok("" | "off") => Ok(None),
        Ok("on") => Ok(Some(FakeProfile)),
        _ => Err(io::Error::other(format!("{PROFILE_VARIABLE} invalid"))),
    }
}

impl FakeProfile {
    /// Every environment assignment this profile makes: each paid capability off at its
    /// existing switch, and the fixed harbor adventure, which needs no generation.
    fn environment(self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("DF_GAMEPLAY_STORYTELLER", "off"),
            ("DF_GAMEPLAY_MEMORY", "off"),
            ("DF_GAMEPLAY_VOICE", "off"),
            ("DF_GAMEPLAY_MUSIC", "off"),
            ("DF_GAMEPLAY_STT", "off"),
            ("DF_GAMEPLAY_IMAGERY", "off"),
            ("DF_GAMEPLAY_PLATES", "off"),
            ("DF_GAMEPLAY_CREATOR_PORTRAITS", "off"),
            ("DF_GAMEPLAY_HOME_VIDEO", "off"),
            ("DF_GAMEPLAY_SCENE_VIDEO", "off"),
            ("DF_GAMEPLAY_CHARACTER_VIDEO", "off"),
            ("DF_GAMEPLAY_JEV", "off"),
            ("DF_GAMEPLAY_SPLAT_WORLDS", "off"),
            ("DF_GAMEPLAY_ADVENTURE", "harbor"),
        ]
    }

    /// The startup audit declares the fake outputs and the disabled external providers.
    pub fn startup_line(self) -> String {
        "FAKE_PROVIDERS ready adventure=harbor text=deterministic audio=silent image=placeholder providers=off".to_owned()
    }
}

/// Applies the selected profile to the process before the runtime starts.
///
/// # Safety
/// No other thread may read or write the environment during this call.
pub unsafe fn apply_from_environment() -> Result<Option<FakeProfile>, io::Error> {
    let profile = parse_profile(std::env::var(PROFILE_VARIABLE))?;
    if let Some(profile) = profile {
        let listen = std::env::var("DF_GAMEPLAY_LISTEN")
            .map_err(|_| io::Error::other("fake profile needs an explicit loopback listener"))?;
        validate_listener(&listen)?;
        for (name, value) in profile.environment() {
            // SAFETY: called once from `main` before the runtime or any other thread starts,
            // so no concurrent environment access can race these writes.
            unsafe { std::env::set_var(name, value) };
        }
        ENABLED.store(true, Ordering::Relaxed);
    }
    Ok(profile)
}

fn validate_listener(value: &str) -> Result<(), io::Error> {
    let address: std::net::SocketAddr = value
        .parse()
        .map_err(|_| io::Error::other("fake profile listener invalid"))?;
    if !address.ip().is_loopback() || address.port() == 0 || address.port() == 63309 {
        return Err(io::Error::other("fake profile listener refused"));
    }
    Ok(())
}

/// Two seconds of silent 8 kHz mono 16-bit audio as a complete WAV file.
fn silent_audio() -> Vec<u8> {
    const SAMPLES: u32 = 16_000;
    let mut bytes = Vec::with_capacity(44 + SAMPLES as usize * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + SAMPLES * 2).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&8_000u32.to_le_bytes());
    bytes.extend_from_slice(&16_000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(SAMPLES * 2).to_le_bytes());
    bytes.resize(44 + SAMPLES as usize * 2, 0);
    bytes
}

/// A silent part only for a segment of a registered, canonical voice line.
pub(crate) fn silent_part(text: &str, index: usize) -> Option<Vec<u8>> {
    (!text.trim().is_empty() && index < crate::voice_line::voice_segments(text).len())
        .then(silent_audio)
}

fn placeholder_image() -> image::RgbaImage {
    image::RgbaImage::from_pixel(32, 32, image::Rgba([24, 32, 48, 255]))
}

/// Creates a fresh placeholder tree, refusing an existing root so artwork cannot be overwritten.
pub fn prepare_assets(root: &Path) -> Result<(), io::Error> {
    std::fs::create_dir(root)?;
    let image = placeholder_image();
    for (relative, format) in [
        ("ui/scenes/mara-harbor-v4.png", image::ImageFormat::Png),
        (
            "concept-art/scene-campfire-under-stars.webp",
            image::ImageFormat::WebP,
        ),
        ("concept-art/vell-avatar.webp", image::ImageFormat::WebP),
    ] {
        let path = root.join(relative);
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("fake asset parent missing"))?;
        std::fs::create_dir_all(parent)?;
        image
            .save_with_format(path, format)
            .map_err(io::Error::other)?;
    }
    std::fs::create_dir_all(root.join("splat"))?;
    Ok(())
}

impl FakeProfile {
    /// Replaces only public presentation; authoritative state, offers and receipts stay intact.
    pub(crate) fn project(self, message: &mut df_protocol::common::ViewMessage) {
        use df_protocol::common::view_message::Audience;
        let (scene, journey) = match message.audience.as_mut() {
            Some(Audience::Display(view)) => {
                view.scene_asset = PLACEHOLDER_PATH.to_owned();
                if let Some(graph) = &mut view.scene_graph {
                    if let Some(set) = &mut graph.set {
                        set.asset = view.scene_asset.clone();
                        set.fallback_asset = view.scene_asset.clone();
                    }
                    for actor in &mut graph.actors {
                        actor.art = PORTRAIT_PATH.to_owned();
                        actor.fallback_art = PORTRAIT_PATH.to_owned();
                    }
                }
                (&mut view.scene, &mut view.journey)
            }
            Some(Audience::Player(view)) => (&mut view.scene, &mut view.journey),
            None => return,
        };
        if let Some(scene) = scene {
            scene.scene_asset = PLACEHOLDER_PATH.to_owned();
            scene.speaker_portrait = PORTRAIT_PATH.to_owned();
            for actor in &mut scene.cast {
                actor.sprite = PORTRAIT_PATH.to_owned();
                actor.portrait = PORTRAIT_PATH.to_owned();
                actor.live_sprite = PORTRAIT_PATH.to_owned();
            }
        }
        if let Some(journey) = journey {
            for member in &mut journey.party {
                member.portrait = PORTRAIT_PATH.to_owned();
            }
            for face in &mut journey.live_faces {
                face.sprite = PORTRAIT_PATH.to_owned();
                face.battle_sprite = PORTRAIT_PATH.to_owned();
            }
            if let Some(combat) = &mut journey.combat {
                for member in &mut combat.participants {
                    member.portrait = PORTRAIT_PATH.to_owned();
                }
            }
            if let Some(character) = &mut journey.own_character {
                character.portrait_uri = PORTRAIT_PATH.to_owned();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_parts_exist_only_for_canonical_line_segments() {
        assert!(silent_part("A quiet moment.", 0).is_some());
        assert!(silent_part("A quiet moment.", 1).is_none());
        assert!(silent_part("", 0).is_none());
    }

    #[test]
    fn fake_profile_refuses_public_zero_and_reserved_listeners() {
        for address in ["0.0.0.0:63411", "127.0.0.1:63309", "127.0.0.1:0", "invalid"] {
            assert!(validate_listener(address).is_err());
        }
        assert!(validate_listener("127.0.0.1:63411").is_ok());
    }

    #[test]
    fn presentation_fakes_preserve_offers_private_data_and_stable_line_identity() {
        use df_protocol::common as rpc;
        let mut message = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    private_clue: "private fixture".to_owned(),
                    offers: vec![rpc::GameplayActionOffer {
                        offer_id: "offer-1".to_owned(),
                        ..Default::default()
                    }],
                    scene: Some(rpc::GameplayScene {
                        speaker_portrait: "/missing-speaker.png".to_owned(),
                        cast: vec![rpc::SceneActor {
                            sprite: "/missing-sprite.png".to_owned(),
                            portrait: "/missing-npc.png".to_owned(),
                            live_sprite: "/missing-live.png".to_owned(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    journey: Some(rpc::JourneyView {
                        party: vec![rpc::PartyMemberView {
                            portrait: "/missing-party.png".to_owned(),
                            ..Default::default()
                        }],
                        live_faces: vec![rpc::LiveFace {
                            sprite: "/missing-face.png".to_owned(),
                            battle_sprite: "/missing-battle.png".to_owned(),
                            ..Default::default()
                        }],
                        combat: Some(rpc::CombatView {
                            participants: vec![rpc::PartyMemberView {
                                portrait: "/missing-combat.png".to_owned(),
                                ..Default::default()
                            }],
                            ..Default::default()
                        }),
                        own_character: Some(rpc::CharacterSheet {
                            portrait_uri: "/missing-sheet.png".to_owned(),
                            ..Default::default()
                        }),
                        table_log: vec![rpc::TableLine {
                            sequence: 17,
                            role: rpc::TableRole::Narrator as i32,
                            text: "original".to_owned(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        FakeProfile.project(&mut message);
        let Some(rpc::view_message::Audience::Player(view)) = &message.audience else {
            panic!("player audience lost");
        };
        assert_eq!(view.private_clue, "private fixture");
        assert_eq!(view.offers[0].offer_id, "offer-1");
        let scene = view.scene.as_ref().unwrap();
        assert_eq!(scene.scene_asset, PLACEHOLDER_PATH);
        for portrait in [
            &scene.speaker_portrait,
            &scene.cast[0].sprite,
            &scene.cast[0].portrait,
            &scene.cast[0].live_sprite,
        ] {
            assert_eq!(portrait, PORTRAIT_PATH);
            assert_eq!(
                df_render::art_contract::player_hero_image(portrait),
                Some(PORTRAIT_PATH)
            );
        }
        let journey = view.journey.as_ref().unwrap();
        assert_eq!(
            df_render::art_contract::player_hero_image(PORTRAIT_PATH),
            Some(PORTRAIT_PATH)
        );
        assert_eq!(journey.party[0].portrait, PORTRAIT_PATH);
        assert_eq!(journey.live_faces[0].sprite, PORTRAIT_PATH);
        assert_eq!(journey.live_faces[0].battle_sprite, PORTRAIT_PATH);
        assert_eq!(
            journey.combat.as_ref().unwrap().participants[0].portrait,
            PORTRAIT_PATH
        );
        assert_eq!(
            journey.own_character.as_ref().unwrap().portrait_uri,
            PORTRAIT_PATH
        );
        assert_eq!(view.journey.as_ref().unwrap().table_log[0].text, "original");
        message.revision = Some(rpc::SessionRevision {
            sequence: Some(999),
            ..Default::default()
        });
        FakeProfile.project(&mut message);
        let Some(rpc::view_message::Audience::Player(view)) = &message.audience else {
            panic!("player audience lost");
        };
        assert_eq!(view.journey.as_ref().unwrap().table_log[0].text, "original");
        message.audience = Some(rpc::view_message::Audience::Display(
            rpc::DisplayGameplayView {
                scene_graph: Some(rpc::DisplaySceneGraph {
                    set: Some(rpc::DisplaySceneSet::default()),
                    actors: vec![rpc::DisplaySceneActor::default()],
                    ..Default::default()
                }),
                ..Default::default()
            },
        ));
        FakeProfile.project(&mut message);
        let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
            panic!("display audience lost");
        };
        assert_eq!(display.scene_asset, PLACEHOLDER_PATH);
        let graph = display.scene_graph.as_ref().unwrap();
        assert_eq!(graph.set.as_ref().unwrap().asset, PLACEHOLDER_PATH);
        assert_eq!(graph.set.as_ref().unwrap().fallback_asset, PLACEHOLDER_PATH);
        for portrait in [&graph.actors[0].art, &graph.actors[0].fallback_art] {
            assert_eq!(portrait, PORTRAIT_PATH);
            assert_eq!(
                df_render::art_contract::player_hero_image(portrait),
                Some(PORTRAIT_PATH)
            );
        }
    }

    #[test]
    fn only_on_selects_the_profile_and_bad_values_are_refused_without_echo() {
        assert_eq!(parse_profile(Err(VarError::NotPresent)).unwrap(), None);
        assert_eq!(parse_profile(Ok("off".to_owned())).unwrap(), None);
        assert_eq!(
            parse_profile(Ok("on".to_owned())).unwrap(),
            Some(FakeProfile)
        );
        for value in ["ON", "true", "private-secret"] {
            let error = parse_profile(Ok(value.to_owned())).unwrap_err();
            assert!(!error.to_string().contains(value));
        }
    }

    #[test]
    fn the_profile_switches_every_paid_capability_off() {
        let environment = FakeProfile.environment();
        assert_eq!(environment.len(), 14);
        assert!(
            environment
                .iter()
                .all(|(_, value)| *value == "off" || *value == "harbor")
        );
        assert!(environment.contains(&("DF_GAMEPLAY_STORYTELLER", "off")));
        assert!(environment.contains(&("DF_GAMEPLAY_JEV", "off")));
        assert!(environment.contains(&("DF_GAMEPLAY_ADVENTURE", "harbor")));
    }

    #[test]
    fn fake_media_are_deterministic_and_well_formed() {
        assert_eq!(silent_audio(), silent_audio());
        let audio = silent_audio();
        assert_eq!(&audio[0..4], b"RIFF");
        assert_eq!(&audio[8..12], b"WAVE");
        assert!(audio[44..].iter().all(|sample| *sample == 0));
        let mut encoded = std::io::Cursor::new(Vec::new());
        placeholder_image()
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        assert_eq!(&encoded.get_ref()[0..4], b"\x89PNG");
        assert_eq!(
            image::load_from_memory(encoded.get_ref()).unwrap().width(),
            32
        );
        assert_eq!(
            FakeProfile.startup_line(),
            "FAKE_PROVIDERS ready adventure=harbor text=deterministic audio=silent image=placeholder providers=off"
        );
    }
}
