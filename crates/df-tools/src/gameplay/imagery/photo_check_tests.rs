use std::io::Cursor;

use crate::gameplay::imagery::SceneParty;
use df_providers::PhotoJudgeKind;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::*;

fn image_jpeg() -> Vec<u8> {
    let image = image::RgbImage::from_pixel(2, 2, image::Rgb([90, 100, 110]));
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut encoded, image::ImageFormat::Jpeg)
        .unwrap();
    encoded.into_inner()
}

fn answer(frame: &str, medium: &str, text: &str, dress: &str, people_count: Option<&str>) -> Value {
    let mut answer = json!({
        "frame": frame,
        "medium": medium,
        "text": text,
        "dress": dress,
        "setting": "period",
    });
    if let Some(people_count) = people_count {
        answer["people_count"] = json!(people_count);
    }
    answer
}

fn scene_answer(heroes: u64) -> Value {
    let mut value = answer("single_photograph", "photograph", "none", "period", None);
    value["hero_count"] = json!(heroes);
    value["weapon_custody"] = json!("matches_custody");
    value["visible_features"] = json!([]);
    value
}

async fn fake_judge_endpoint(
    answer: Value,
) -> (std::net::SocketAddr, tokio::sync::oneshot::Receiver<Value>) {
    fake_judgments_endpoint(vec![answer]).await
}
async fn fake_judgments_endpoint(
    answers: Vec<Value>,
) -> (std::net::SocketAddr, tokio::sync::oneshot::Receiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (request_sender, request_receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let mut request_sender = Some(request_sender);
        for answer in answers {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let body_length = loop {
                let mut chunk = [0_u8; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                assert_ne!(read, 0);
                request.extend_from_slice(&chunk[..read]);
                if let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    break headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                }
            };
            loop {
                let header_end = request
                    .windows(4)
                    .position(|bytes| bytes == b"\r\n\r\n")
                    .unwrap()
                    + 4;
                if request.len() >= header_end + body_length {
                    break;
                }
                let mut chunk = [0_u8; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                assert_ne!(read, 0);
                request.extend_from_slice(&chunk[..read]);
            }
            let header_end = request
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap()
                + 4;
            let request_body = serde_json::from_slice(&request[header_end..]).unwrap();
            if let Some(sender) = request_sender.take() {
                let _ = sender.send(request_body);
            }
            let outer = json!({
                "choices": [{"message": {"content": answer.to_string()}}],
                "usage": {"prompt_tokens": 12, "completion_tokens": 8},
            });
            let body = outer.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (address, request_receiver)
}

#[test]
fn photographic_correction_preserves_an_empty_rooms_people_count() {
    let correction = PhotoCheckFailure::NotPhotograph.correction();
    assert!(correction.contains("exactly zero people for an empty place"));
    assert!(correction.contains("Clean surfaces and even daylight are welcome"));
    assert!(!correction.contains("real people"));
}

struct SavedPhotoProof(std::path::PathBuf);
impl super::super::DevelopmentSavedPhotoSource for SavedPhotoProof {
    fn read_current<'a>(
        &'a self,
        owner: &'a df_imagery::ImageOwner,
    ) -> super::super::central::ReferenceFuture<'a, ReferenceImage> {
        Box::pin(async move {
            if !matches!(owner, df_imagery::ImageOwner::Run { .. }) {
                return Err(super::super::ImageFailure::Refused);
            }
            super::super::Imagery::read_saved_photo(&self.0, 16 * 1024 * 1024)
                .await
                .map_err(|_| super::super::ImageFailure::BadOutput)
        })
    }
}
async fn saved_check(
    imagery: &super::super::Imagery,
    kind: PhotoJudgeKind,
    path: &std::path::Path,
) -> Result<PhotoCheckOutcome, PhotoCheckError> {
    imagery
        .check_saved_photo(
            kind,
            path,
            std::sync::Arc::new(SavedPhotoProof(path.to_owned())),
        )
        .await
}

#[tokio::test]
async fn saved_photo_check_calls_the_judge_once_with_the_files_picture_and_kind() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let directory = crate::gameplay::imagery::tests::directory("saved-photo-check");
    std::fs::create_dir_all(&directory).unwrap();
    let bytes = image_jpeg();
    let path = directory.join("saved candidate.image");
    std::fs::write(&path, &bytes).unwrap();
    let Some(mut fixture) = DirectionFixture::photographic(&directory).await else {
        return;
    };
    for kind in [
        PhotoCheckKind::CanonicalRoom,
        PhotoCheckKind::ScenePlate,
        PhotoCheckKind::CanonicalCharacter,
    ] {
        let mut reply = answer("single_photograph", "photograph", "none", "period", None);
        match kind {
            PhotoCheckKind::CanonicalRoom => reply["people_present"] = json!("none"),
            PhotoCheckKind::CanonicalCharacter => reply["people_count"] = json!("exactly_one"),
            PhotoCheckKind::ScenePlate => {}
        }
        let (address, saved_request) = fake_judge_endpoint(reply.clone()).await;
        fixture.photo_judge(address).await;
        assert_eq!(
            saved_check(&fixture.imagery, kind.into(), &path)
                .await
                .unwrap(),
            PhotoCheckOutcome::Pass
        );
        let request = saved_request.await.unwrap();
        assert_eq!(request["model"], "gpt-5.5");
        assert_eq!(request["max_completion_tokens"], 3000);
        assert_eq!(
            request["response_format"]["json_schema"]["name"],
            "photo-quality"
        );
        assert_eq!(
            request["messages"][0]["content"][1]["text"],
            "Picture candidate-1:"
        );
        let properties = &request["response_format"]["json_schema"]["schema"]["properties"];
        assert_eq!(
            properties.get("people_present").is_some(),
            kind == PhotoCheckKind::CanonicalRoom
        );
        assert_eq!(
            properties.get("people_count").is_some(),
            kind == PhotoCheckKind::CanonicalCharacter
        );
        // Full legacy JSON and JPEG parity lives in the private imagery adapter golden test.
    }
    assert_eq!(
        fixture.calls(),
        0,
        "saved inspection never generates artwork"
    );
    fixture.close().await;
}
#[tokio::test]
async fn saved_photo_check_reports_the_closed_reason_and_refuses_bad_files_before_judging() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let directory = crate::gameplay::imagery::tests::directory("saved-photo-failure");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("candidate.image");
    std::fs::write(&path, image_jpeg()).unwrap();
    let Some(mut fixture) = DirectionFixture::photographic(&directory).await else {
        return;
    };
    let (address, _) = fake_judge_endpoint(room_answer("none", "modern")).await;
    fixture.photo_judge(address).await;
    let outcome = saved_check(&fixture.imagery, PhotoJudgeKind::CanonicalRoom, &path)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        PhotoCheckOutcome::Fail(PhotoCheckFailure::ModernSetting)
    );
    assert_eq!(outcome.verdict(), "fail");
    assert_eq!(outcome.reason(), Some("modern_setting"));
    let (address, mut requests) = fake_judge_endpoint(room_answer("none", "period")).await;
    fixture.photo_judge(address).await;
    assert_eq!(
        saved_check(
            &fixture.imagery,
            PhotoJudgeKind::CanonicalRoom,
            &directory.join("missing.image")
        )
        .await,
        Err(PhotoCheckError::FileRead)
    );
    std::fs::write(&path, b"not an image").unwrap();
    assert_eq!(
        saved_check(&fixture.imagery, PhotoJudgeKind::ScenePlate, &path).await,
        Err(PhotoCheckError::ImagePreparation)
    );
    let oversized = directory.join("oversized.image");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    assert_eq!(
        saved_check(&fixture.imagery, PhotoJudgeKind::ScenePlate, &oversized).await,
        Err(PhotoCheckError::FileTooLarge)
    );
    assert!(matches!(
        requests.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert_eq!(fixture.calls(), 0);
    fixture.close().await;
}

#[tokio::test]
async fn saved_photo_check_each_explicit_read_is_fresh_and_never_reuses_a_prior_judgement() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let root = crate::gameplay::imagery::tests::directory("saved-photo-fresh");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("source.image");
    std::fs::write(&path, image_jpeg()).unwrap();
    let Some(mut fixture) = DirectionFixture::photographic(&root).await else {
        return;
    };
    let (address, _) = fake_judgments_endpoint(vec![
        room_answer("none", "period"),
        room_answer("none", "modern"),
    ])
    .await;
    fixture.photo_judge(address).await;
    assert_eq!(
        saved_check(&fixture.imagery, PhotoJudgeKind::CanonicalRoom, &path)
            .await
            .unwrap(),
        PhotoCheckOutcome::Pass
    );
    assert_eq!(
        saved_check(&fixture.imagery, PhotoJudgeKind::CanonicalRoom, &path)
            .await
            .unwrap(),
        PhotoCheckOutcome::Fail(PhotoCheckFailure::ModernSetting)
    );
    assert_eq!(fixture.calls(), 0);
    fixture.close().await;
}

#[tokio::test]
async fn saved_photo_check_refuses_source_bytes_changed_during_trusted_import_before_judging() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    struct ChangedSource(std::path::PathBuf);
    impl super::super::DevelopmentSavedPhotoSource for ChangedSource {
        fn read_current<'a>(
            &'a self,
            owner: &'a df_imagery::ImageOwner,
        ) -> super::super::central::ReferenceFuture<'a, ReferenceImage> {
            Box::pin(async move {
                if !matches!(owner, df_imagery::ImageOwner::Run { .. }) {
                    return Err(super::super::ImageFailure::Refused);
                }
                let path = self.0.clone();
                tokio::task::spawn_blocking(move || {
                    let mut changed = std::io::Cursor::new(Vec::new());
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                        2,
                        2,
                        image::Rgb([240, 50, 20]),
                    ))
                    .write_to(&mut changed, image::ImageFormat::Jpeg)
                    .unwrap();
                    std::fs::write(path, changed.into_inner()).unwrap();
                })
                .await
                .unwrap();
                super::super::Imagery::read_saved_photo(&self.0, 16 * 1024 * 1024)
                    .await
                    .map_err(|_| super::super::ImageFailure::BadOutput)
            })
        }
    }
    let root = crate::gameplay::imagery::tests::directory("saved-photo-source-change");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("source.image");
    std::fs::write(&path, image_jpeg()).unwrap();
    let Some(mut fixture) = DirectionFixture::photographic(&root).await else {
        return;
    };
    let (address, mut requests) = fake_judge_endpoint(room_answer("none", "period")).await;
    fixture.photo_judge(address).await;
    assert_eq!(
        fixture
            .imagery
            .check_saved_photo(
                PhotoJudgeKind::CanonicalRoom,
                &path,
                std::sync::Arc::new(ChangedSource(path.clone())),
            )
            .await,
        Err(PhotoCheckError::Analysis(
            super::super::ImageFailure::Refused
        ))
    );
    assert!(matches!(
        requests.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert_eq!(fixture.calls(), 0);
    fixture.close().await;
}

fn room_answer(text: &str, setting: &str) -> Value {
    let mut value = answer("single_photograph", "photograph", text, "no_people", None);
    value["setting"] = json!(setting);
    value["people_present"] = json!("none");
    value
}

async fn canonical_room_request(
    fixture: &crate::gameplay::imagery::tests::DirectionFixture,
) -> super::super::ImageRequest {
    use crate::gameplay::{
        adventure::scene_model::SceneModel, imagery, startup_configuration::ContentRating,
    };
    let model = SceneModel {
        services: Vec::new(),
        witnessed_thefts: Vec::new(),
        inventory_version: 1,
        time_of_day: "morning".to_owned(),
        weather: "clear".to_owned(),
        people: Vec::new(),
        things: Vec::new(),
        ways_out: Vec::new(),
        offers: Vec::new(),
    };
    imagery::ImageRequest {
        kind: imagery::ImageKind::CanonicalRoom,
        references: vec![
            fixture
                .import_bytes(ImageFormat::Jpeg, bytes::Bytes::from(image_jpeg()))
                .await
                .1,
        ],
        prompt: imagery::canonical_room(ContentRating::Restrained, &model, "test-street", "P1")
            .unwrap(),
        output: imagery::OutputSpec::BACKDROP,
    }
}

async fn scene_plate_request(
    fixture: &crate::gameplay::imagery::tests::DirectionFixture,
    heroes: usize,
    edit: bool,
) -> super::super::ImageRequest {
    use crate::gameplay::{imagery, shot::Camera, startup_configuration::ContentRating};
    let party = SceneParty::new(heroes, 2).unwrap();
    let cast = imagery::FluxCast {
        sheet: (1..=heroes)
            .map(|ordinal| imagery::FluxPerson {
                name: format!("Hero {ordinal}"),
                appearance: None,
            })
            .collect(),
        speaker: None,
        portrait: false,
    };
    let prompt = imagery::flux_scene_plate_for_party(
        ContentRating::Restrained,
        Camera::WideEstablishing,
        None,
        &cast,
        "{}",
        None,
        heroes,
    )
    .unwrap()
    .with_development_direction("Keep the established action")
    .with_cast_sheet(2, "Heroes in seat order")
    .with_focus_actor("hero-1", "Hero 1");
    assert_eq!(prompt.scene_party(), Some(party));
    imagery::ImageRequest {
        kind: if edit {
            imagery::ImageKind::ScenePlateEdit
        } else {
            imagery::ImageKind::ScenePlate
        },
        references: vec![
            fixture
                .import_bytes(ImageFormat::Jpeg, image_jpeg().into())
                .await
                .1,
            fixture
                .import_bytes(ImageFormat::Jpeg, image_jpeg().into())
                .await
                .1,
        ],
        prompt,
        output: imagery::OutputSpec::PLATE,
    }
}

#[tokio::test]
async fn wrong_scene_hero_count_retries_and_right_count_passes_for_each_party_size() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    for heroes in [1, 2, 4, 6] {
        for edit in [false, true] {
            for retry in [false, true] {
                let root = crate::gameplay::imagery::tests::directory("scene-count-retry");
                let Some(mut fixture) = DirectionFixture::photographic(&root).await else {
                    return;
                };
                let mut answers = Vec::new();
                if retry {
                    answers.push(scene_answer(heroes as u64 + 1));
                }
                answers.push(scene_answer(heroes as u64));
                let (judge, request_seen) = fake_judgments_endpoint(answers).await;
                fixture.photo_judge(judge).await;
                let request = scene_plate_request(&fixture, heroes, edit).await;
                let original = request.prompt.as_str().to_owned();
                let key = fixture
                    .imagery
                    .generate("plate:test-room", request)
                    .await
                    .unwrap();
                assert_eq!(fixture.calls(), if retry { 2 } else { 1 });
                assert!(fixture.prompt().contains(&original));
                if retry {
                    assert!(
                        fixture.prompt().contains(
                            PhotoCheckFailure::HeroCount {
                                expected: heroes as u8,
                                seen: heroes as u64 + 1
                            }
                            .correction()
                        )
                    );
                }
                assert!(matches!(
                    fixture.imagery.image(key.as_str()).await,
                    imagery::Lookup::Image { .. }
                ));
                let request_seen = request_seen.await.unwrap();
                let instruction = request_seen["messages"][0]["content"][0]["text"]
                    .as_str()
                    .unwrap();
                assert!(instruction.contains(&format!("exactly {heroes} player heroes")));
                fixture.close().await;
            }
        }
    }
}

#[tokio::test]
async fn persistent_scene_count_mismatch_keeps_the_fallback_and_never_publishes_rejected_bytes() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    for edit in [false, true] {
        let root = crate::gameplay::imagery::tests::directory("scene-count-refused");
        let Some(mut fixture) = DirectionFixture::photographic(&root).await else {
            return;
        };
        let mut wrong = scene_answer(7);
        wrong["text"] = json!("visible_text");
        let (judge, _) = fake_judgments_endpoint(vec![wrong.clone(), wrong]).await;
        fixture.photo_judge(judge).await;
        let key = fixture
            .imagery
            .key_for(&scene_plate_request(&fixture, 6, edit).await);
        for _ in 0..2 {
            assert_eq!(
                fixture
                    .imagery
                    .generate(
                        "plate:test-room",
                        scene_plate_request(&fixture, 6, edit).await
                    )
                    .await,
                Err(imagery::ImageFailure::BadOutput)
            );
        }
        assert_eq!(
            fixture.calls(),
            2,
            "one corrected regeneration per refused batch"
        );
        assert!(!matches!(
            fixture.imagery.image(key.as_str()).await,
            imagery::Lookup::Image { .. }
        ));
        assert!(
            fixture
                .imagery
                .peek(&scene_plate_request(&fixture, 6, edit).await)
                .await
                .is_none()
        );

        fixture.close().await;
    }
}

#[tokio::test]
async fn failing_then_passing_room_uses_the_judges_reason_in_the_second_generation_prompt() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS16/room-retry-pass")
        .join(std::process::id().to_string());
    let Some(mut fixture) = DirectionFixture::photographic(&root).await else {
        return;
    };
    let (judge, _) = fake_judgments_endpoint(vec![
        room_answer("visible_text", "period"),
        room_answer("none", "period"),
    ])
    .await;
    fixture.photo_judge(judge).await;
    let request = canonical_room_request(&fixture).await;
    let original = request.prompt.as_str().to_owned();
    let key = fixture
        .imagery
        .generate("place:test-street", request)
        .await
        .unwrap();
    assert_eq!(fixture.calls(), 2);
    assert!(
        fixture
            .prompt()
            .contains(PhotoCheckFailure::VisibleText.correction())
    );
    assert!(fixture.prompt().contains(&original));
    assert!(matches!(
        fixture.imagery.image(key.as_str()).await,
        imagery::Lookup::Image { .. }
    ));
    fixture.close().await;
}

#[tokio::test]
async fn persistently_rejected_room_stops_at_two_generations_until_readmission() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS16/room-retry-bound")
        .join(std::process::id().to_string());
    let Some(mut fixture) = DirectionFixture::photographic(&root).await else {
        return;
    };
    let (judge, _) = fake_judgments_endpoint(vec![
        room_answer("none", "modern"),
        room_answer("none", "modern"),
    ])
    .await;
    fixture.photo_judge(judge).await;
    for _ in 0..2 {
        assert_eq!(
            fixture
                .imagery
                .generate("place:test-street", canonical_room_request(&fixture).await)
                .await,
            Err(imagery::ImageFailure::BadOutput)
        );
    }
    assert_eq!(
        fixture.calls(),
        2,
        "a batch has exactly one corrected regeneration"
    );
    assert!(
        fixture
            .prompt()
            .contains(PhotoCheckFailure::ModernSetting.correction())
    );
    fixture.close().await;
}

#[tokio::test]
async fn unexpected_drawn_weapon_uses_the_existing_plate_regeneration_gate() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    for edit in [false, true] {
        let root = crate::gameplay::imagery::tests::directory("unexpected-weapon");
        let Some(mut fixture) = DirectionFixture::photographic(&root).await else {
            return;
        };
        let mut wrong = scene_answer(6);
        wrong["weapon_custody"] = json!("unexpected_drawn_weapon");
        let (judge, _) = fake_judgments_endpoint(vec![wrong, scene_answer(6)]).await;
        fixture.photo_judge(judge).await;
        let key = fixture
            .imagery
            .generate(
                "plate:weapon-custody",
                scene_plate_request(&fixture, 6, edit).await,
            )
            .await
            .unwrap();
        assert_eq!(fixture.calls(), 2);
        assert!(
            fixture
                .prompt()
                .contains(PhotoCheckFailure::UnexpectedDrawnWeapon.correction())
        );
        assert!(matches!(
            fixture.imagery.image(key.as_str()).await,
            imagery::Lookup::Image { .. }
        ));
        fixture.close().await;
    }
}

#[tokio::test]
async fn production_scene_gate_receives_compact_custody_for_fresh_and_edited_plates() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let custody =
        "Vesper: sword drawn in right hand, left hand empty. Other heroes: both hands empty.";
    for edit in [false, true] {
        let cache = crate::gameplay::imagery::tests::directory("plate-custody");
        let Some(mut fixture) = DirectionFixture::photographic(&cache).await else {
            return;
        };
        let (judge, seen) = fake_judge_endpoint(scene_answer(6)).await;
        fixture.photo_judge(judge).await;
        let mut request = scene_plate_request(&fixture, 6, edit).await;
        request.prompt = request.prompt.with_scene_hand_custody(custody.into());
        fixture
            .imagery
            .generate("plate:HS37-production-custody", request)
            .await
            .unwrap();
        let seen = seen.await.unwrap();
        let instruction = seen["messages"][0]["content"][0]["text"].as_str().unwrap();
        assert!(instruction.contains(custody), "{instruction}");
        assert!(instruction.chars().count() <= 6_000);
        assert_eq!(fixture.calls(), 1);
        fixture.close().await;
    }
}

#[tokio::test]
async fn photo_check_keeps_only_the_accepted_retry_features_with_the_cached_image_key() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let cache = crate::gameplay::imagery::tests::directory("photo_check_keeps_only_the_accepted_");
    let Some(mut fixture) = DirectionFixture::photographic(&cache).await else {
        return;
    };
    let mut accepted = scene_answer(6);
    accepted["visible_features"] = json!([{"name":"fruit stand","look":"Apples under canvas.","region":[600,200,900,700],"recorded_id":null}]);
    let mut rejected = accepted.clone();
    rejected["hero_count"] = json!(5);
    rejected["visible_features"][0]["name"] = json!("rejected fish cart");
    let (judge, sent) = fake_judgments_endpoint(vec![rejected, accepted]).await;
    fixture.photo_judge(judge).await;
    let mut request = scene_plate_request(&fixture, 6, false).await;
    request.prompt = request.prompt.with_scene_hand_custody(
        json!({"hands":"Hands empty; weapons sheathed.",
        "place":"place-market-street","source_hash":"f".repeat(64),"public_records":[]})
        .to_string(),
    );
    let mut other_source = request.clone();
    other_source.prompt = other_source.prompt.with_scene_hand_custody(
        json!({"hands":"Hands empty; weapons sheathed.",
        "place":"place-market-street","source_hash":"e".repeat(64),"public_records":[]})
        .to_string(),
    );
    assert_ne!(
        fixture.imagery.key_for(&request),
        fixture.imagery.key_for(&other_source)
    );
    let key = fixture
        .imagery
        .generate("plate:HS40-accepted-evidence", request.clone())
        .await
        .unwrap();
    let evidence = fixture.imagery.photo_evidence(&key).await.unwrap().unwrap();
    assert_eq!(evidence.place, "place-market-street");
    assert_eq!(evidence.source_hash, "f".repeat(64));
    assert_eq!(evidence.visible_features.len(), 1);
    assert_eq!(evidence.visible_features[0].name, "fruit stand");
    let cached = fixture
        .imagery
        .generate("plate:HS40-accepted-evidence", request)
        .await
        .unwrap();
    assert_eq!(cached, key);
    assert_eq!(fixture.calls(), 2);
    assert_eq!(
        fixture
            .imagery
            .photo_evidence(&cached)
            .await
            .unwrap()
            .unwrap()
            .visible_features,
        evidence.visible_features
    );
    sent.await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn photo_features_without_a_bound_place_do_not_break_legacy_plate_requests() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS40/photo-unbound-cache")
        .join(std::process::id().to_string());
    let Some(mut fixture) = DirectionFixture::photographic(&cache).await else {
        return;
    };
    let mut accepted = scene_answer(6);
    accepted["visible_features"] = json!([{"name":"fruit stand","look":"Apples under canvas.","region":[600,200,900,700],"recorded_id":null}]);
    let (judge, sent) = fake_judge_endpoint(accepted).await;
    fixture.photo_judge(judge).await;
    let mut request = scene_plate_request(&fixture, 6, false).await;
    request.prompt = request
        .prompt
        .with_scene_hand_custody("Hands empty; weapons sheathed.".into());
    let key = fixture
        .imagery
        .generate("plate:HS40-unbound-evidence", request)
        .await
        .unwrap();
    assert!(
        fixture
            .imagery
            .photo_evidence(&key)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.calls(), 1);
    sent.await.unwrap();
    fixture.close().await;
}
