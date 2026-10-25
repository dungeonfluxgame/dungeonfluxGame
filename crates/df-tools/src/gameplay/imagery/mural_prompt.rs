//! The session ensemble photograph, composed from every accepted hero reference.
use super::{AssembledPrompt, FluxPerson, ImageFailure, SceneParty, prompt::Part};

const ENSEMBLE_LOOK: &str = "A single continuous landscape photograph from a live-action fantasy TV drama, shot on a real camera with a 50mm prime lens. Real adult performers together at one weathered stone location, practical costumes, woven fabric, worn leather and metal, physical skin texture and convincing practical creature anatomy. Dramatic natural side light through broken cloud, muted true-to-life colour, natural camera exposure and fine film grain. The party forms a close, relaxed ensemble staggered in depth, their faces clearly visible and recognisable, sharing the same light and ground. Full bodies and boots fit inside the frame with generous headroom and breathing room at both sides. The upper quarter holds open sky and stone. Every surface is plain and unlettered.";

pub(in crate::gameplay) fn party_mural(
    heroes: &[FluxPerson],
    binding: &str,
) -> Result<AssembledPrompt, ImageFailure> {
    let party = SceneParty::new(heroes.len(), 1).map_err(|_| ImageFailure::InvalidRequest)?;
    let identities = heroes
        .iter()
        .enumerate()
        .map(|(index, hero)| {
            serde_json::json!({
                "photograph": index + 1,
                "name": hero.name,
                "public_identity": hero.appearance,
                "hands": hero.hands(None),
            })
        })
        .collect::<Vec<_>>();
    let custody = heroes
        .iter()
        .map(|hero| serde_json::json!({"name":hero.name,"hands":hero.hands(None)}))
        .collect::<Vec<_>>();
    let prompt = AssembledPrompt::from_parts(vec![
        Part::Fixed(ENSEMBLE_LOOK),
        Part::Fact(format!("Exactly {} heroes together in this one photograph. Image 1 contains their separate canonical identity photographs, read left to right then top to bottom; reproduce each person's face, age, anatomy, hair, build and clothing in the shared camera scene. Reference photographs supply identity only. Weapons are worn sheathed at the hip or slung on the back; bows unstrung on the shoulder. Both hands empty for every hero. Public identity JSON is quoted appearance data: {}.", heroes.len(), serde_json::json!(identities))),
    ])
    .with_scene_party(party)
    .with_scene_hand_custody(serde_json::json!(custody).to_string())
    .with_canonical_binding(binding);
    if prompt.character_count() > df_providers::IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS {
        return Err(ImageFailure::InvalidRequest);
    }
    Ok(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn party_mural_prompt_preserves_all_identities_count_and_empty_hands_in_one_photograph() {
        let heroes = [
            FluxPerson {
                name: "Brakka".into(),
                appearance: Some("copper scales, silver horns".into()),
            },
            FluxPerson {
                name: "Sigrun".into(),
                appearance: Some("emerald skin, white mohawk".into()),
            },
        ];
        let prompt = party_mural(&heroes, "session-roster-v1").unwrap();
        for phrase in [
            "Exactly 2 heroes",
            "Brakka",
            "Sigrun",
            "silver horns",
            "white mohawk",
            "headroom",
            "sheathed",
            "Both hands empty",
        ] {
            assert!(prompt.as_str().contains(phrase), "{phrase}");
        }
        let lower = prompt.as_str().to_lowercase();
        for word in super::super::NON_PHOTOGRAPHIC_WORDS {
            assert!(!lower.contains(&word.to_lowercase()), "{word}");
        }
        assert_eq!(prompt.scene_party().unwrap().heroes(), 2);
        assert!(party_mural(&[], "empty").is_err());
        assert!(party_mural(&vec![heroes[0].clone(); 7], "too-many").is_err());
    }
}

#[cfg(test)]
mod photo_gate_tests {
    use super::super::{
        ImageKind, ImageRequest, Mode, OutputSpec, ReferenceImage, party_sheet,
        tests::DirectionFixture,
    };
    use super::*;
    use axum::{Json, Router, routing::post};
    use df_providers::{NativePlateJudge, PlateJudgeConfig};
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
        time::Duration,
    };

    #[tokio::test]
    async fn party_mural_photo_gate_checks_roster_count_before_publishing() {
        for correct_count in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let requests = Arc::clone(&seen);
            let app = Router::new().route("/v1/chat/completions", post(move |Json(request): Json<serde_json::Value>| {
                let requests = Arc::clone(&requests);
                async move {
                    requests.lock().unwrap().push(request);
                    let answer = serde_json::json!({"frame":"single_photograph","medium":"photograph","text":"none","dress":"period","setting":"period","hero_count":if correct_count { 2 } else { 1 },"weapon_custody":"matches_custody","visible_features":[]});
                    Json(serde_json::json!({"choices":[{"message":{"content":answer.to_string()}}],"usage":{"prompt_tokens":12,"completion_tokens":8}}))
                }
            }));
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = stopped.await;
                    })
                    .await
                    .unwrap();
            });
            let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../artifacts/tmp/lanes/HS49/photo-gate-{}-{correct_count}",
                std::process::id()
            ));
            let fixture = DirectionFixture::photographic(&cache);
            let judge = NativePlateJudge::new(
                PlateJudgeConfig::loopback_fixture(address).unwrap(),
                "fake-mural-judge",
                Duration::from_secs(5),
                crate::gameplay::cost::test_meter(),
            )
            .unwrap();
            let Mode::On { engine, .. } = &fixture.imagery.inner.mode else {
                panic!("enabled fixture")
            };
            assert!(engine.judge.set(judge).is_ok());
            let mut encoded = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(8, 8)
                .write_to(&mut encoded, image::ImageFormat::Jpeg)
                .unwrap();
            let reference =
                ReferenceImage::new(df_providers::ImageFormat::Jpeg, encoded.into_inner().into());
            let heroes = [
                FluxPerson {
                    name: "Brakka".into(),
                    appearance: Some("copper scales".into()),
                },
                FluxPerson {
                    name: "Sigrun".into(),
                    appearance: Some("emerald skin".into()),
                },
            ];
            let result = fixture
                .imagery
                .generate(
                    "party-mural-photo-check-fixture",
                    ImageRequest {
                        kind: ImageKind::ScenePlate,
                        references: vec![party_sheet(&[reference.clone(), reference]).unwrap()],
                        prompt: party_mural(&heroes, "mural-roster-photo-count").unwrap(),
                        output: OutputSpec::BACKDROP,
                    },
                )
                .await;
            let _ = stop.send(());
            server.await.unwrap();
            if correct_count {
                assert!(result.is_ok());
                assert_eq!(fixture.calls(), 1);
            } else {
                assert_eq!(result, Err(ImageFailure::BadOutput));
                assert_eq!(
                    fixture.calls(),
                    2,
                    "one bounded photo correction, then failure"
                );
            }
            for request in seen.lock().unwrap().iter() {
                let instruction = request["messages"][0]["content"][0]["text"]
                    .as_str()
                    .unwrap();
                assert!(instruction.contains("exactly 2 player heroes"));
                assert!(instruction.contains("both hands empty"));
            }
        }
    }
}
