use std::{io::Cursor, time::Duration};

use df_providers::PlateJudgeConfig;
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

async fn fake_judge(answer: Value) -> (NativePlateJudge, tokio::sync::oneshot::Receiver<Value>) {
    fake_judgments(vec![answer]).await
}

fn scene_answer(heroes: u64) -> Value {
    let mut value = answer("single_photograph", "photograph", "none", "period", None);
    value["hero_count"] = json!(heroes);
    value["weapon_custody"] = json!("matches_custody");
    value["visible_features"] = json!([]);
    value
}

#[tokio::test]
async fn photo_check_reads_visible_features_in_the_same_quality_judgement() {
    let mut answer = scene_answer(6);
    answer["visible_features"] = json!([{
        "name": "fruit stand", "look": "A canvas stall with red apples in shallow baskets.",
        "region": [650, 220, 920, 600], "recorded_id": null,
    }]);
    let (judge, request) = fake_judge(answer).await;
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg().into());
    let reading = super::super::Imagery::read_scene_photo(
        &judge,
        &image_jpeg(),
        ImageFormat::Jpeg,
        SceneParty::new(6, 2).unwrap(),
        &sheet,
        "hands empty",
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(reading.outcome, PhotoCheckOutcome::Pass);
    assert_eq!(reading.visible_features.len(), 1);
    assert_eq!(reading.visible_features[0].name, "fruit stand");
    assert_eq!(reading.visible_features[0].region, [650, 220, 920, 600]);
    let request = request.await.unwrap();
    let schema = &request["response_format"]["json_schema"]["schema"];
    assert_eq!(schema["properties"]["visible_features"]["maxItems"], 8);
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("visible_features"))
    );
}

#[tokio::test]
async fn photo_check_rejects_invalid_regions_and_never_exports_rejected_picture_features() {
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg().into());
    for (heroes, region, expected_error) in [
        (6, json!([600, 200, 500, 400]), true),
        (6, json!([0, 0, 1001, 1000]), true),
        (5, json!([0, 0, 1000, 1000]), false),
    ] {
        let mut answer = scene_answer(heroes);
        answer["visible_features"] = json!([{"name":"fruit stand", "look":"A distant canvas stall.",
            "region":region, "recorded_id":null}]);
        let (judge, _) = fake_judge(answer).await;
        let result = super::super::Imagery::read_scene_photo(
            &judge,
            &image_jpeg(),
            ImageFormat::Jpeg,
            SceneParty::new(6, 2).unwrap(),
            &sheet,
            "",
            Duration::from_secs(5),
        )
        .await;
        if expected_error {
            assert!(matches!(
                result,
                Err(PhotoCheckError::Judge(PlateJudgeError::Output))
            ));
        } else {
            let reading = result.unwrap();
            assert!(matches!(
                reading.outcome,
                PhotoCheckOutcome::Fail(PhotoCheckFailure::HeroCount { .. })
            ));
            assert!(reading.visible_features.is_empty());
        }
    }
}

#[tokio::test]
async fn scene_photo_counts_against_the_sheet_and_requires_the_actual_count() {
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg().into());
    for expected in [1, 2, 4, 6] {
        for seen in [0, expected, expected + 1] {
            let (judge, request) = fake_judge(scene_answer(seen as u64)).await;
            let outcome = check_scene_photo(
                &judge,
                &image_jpeg(),
                ImageFormat::Jpeg,
                SceneParty::new(expected, 2).unwrap(),
                &sheet,
                Duration::from_secs(5),
            )
            .await
            .unwrap();
            assert_eq!(
                outcome,
                if seen == expected {
                    PhotoCheckOutcome::Pass
                } else {
                    PhotoCheckOutcome::Fail(PhotoCheckFailure::HeroCount {
                        expected: expected as u8,
                        seen: seen as u64,
                    })
                }
            );
            let request = request.await.unwrap();
            let content = request["messages"][0]["content"].as_array().unwrap();
            let instruction = content[0]["text"].as_str().unwrap();
            assert!(instruction.contains(&format!("exactly {expected} player heroes")));
            assert!(instruction.contains("including partly hidden figures, duplicates"));
            assert!(instruction.contains("later photographs identify NPCs"));
            assert_eq!(content[1]["text"], "Picture candidate-1:");
            assert_eq!(content[3]["text"], "Picture reference-sheet:");
            assert_eq!(
                content
                    .iter()
                    .filter(|part| part["type"] == "image_url")
                    .count(),
                2
            );
            let schema = &request["response_format"]["json_schema"]["schema"];
            assert_eq!(schema["properties"]["hero_count"]["type"], "integer");
            assert!(
                schema["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("hero_count"))
            );
            assert_eq!(schema["additionalProperties"], false);
        }
    }
}

#[tokio::test]
async fn scene_photo_does_not_allow_text_fallback_to_hide_a_count_failure() {
    let mut missing_with_wrong_count = scene_answer(7);
    missing_with_wrong_count
        .as_object_mut()
        .unwrap()
        .remove("weapon_custody");
    assert_eq!(
        parse_scene_photo_answer(&missing_with_wrong_count, SceneParty::new(6, 2).unwrap()),
        Ok(PhotoCheckOutcome::Fail(PhotoCheckFailure::HeroCount {
            expected: 6,
            seen: 7
        }))
    );
    let mut value = scene_answer(7);
    value["text"] = json!("visible_text");
    let (judge, _) = fake_judge(value).await;
    assert_eq!(
        check_scene_photo(
            &judge,
            &image_jpeg(),
            ImageFormat::Jpeg,
            SceneParty::new(6, 2).unwrap(),
            &ReferenceImage::new(ImageFormat::Jpeg, image_jpeg().into()),
            Duration::from_secs(5),
        )
        .await
        .unwrap(),
        PhotoCheckOutcome::Fail(PhotoCheckFailure::HeroCount {
            expected: 6,
            seen: 7
        })
    );
}

#[test]
fn scene_photo_rejects_missing_or_invalid_counts_and_preserves_quality_failures() {
    let party = SceneParty::new(2, 2).unwrap();
    for invalid in [json!(null), json!(-1), json!("2"), json!(2.5)] {
        let mut value = scene_answer(2);
        value["hero_count"] = invalid;
        assert_eq!(
            parse_scene_photo_answer(&value, party),
            Err(PlateJudgeError::Output)
        );
    }
    let mut missing = scene_answer(2);
    missing.as_object_mut().unwrap().remove("hero_count");
    assert_eq!(
        parse_scene_photo_answer(&missing, party),
        Err(PlateJudgeError::Output)
    );
    for (field, value, failure) in [
        ("frame", "grid_or_collage", PhotoCheckFailure::GridOrCollage),
        (
            "frame",
            "cutout_composite",
            PhotoCheckFailure::CutoutComposite,
        ),
        (
            "medium",
            "painting_or_illustration",
            PhotoCheckFailure::NotPhotograph,
        ),
        ("medium", "cgi_or_render", PhotoCheckFailure::NotPhotograph),
        ("text", "visible_text", PhotoCheckFailure::VisibleText),
        (
            "dress",
            "modern_clothing",
            PhotoCheckFailure::ModernClothing,
        ),
        ("setting", "modern", PhotoCheckFailure::ModernSetting),
    ] {
        let mut answer = scene_answer(2);
        answer[field] = json!(value);
        assert_eq!(
            parse_scene_photo_answer(&answer, party),
            Ok(PhotoCheckOutcome::Fail(failure))
        );
    }
}

#[test]
fn hero_count_failure_retains_the_still_and_logs_only_numeric_counts() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS16/count-retention")
        .join(std::process::id().to_string());
    let failure = PhotoCheckFailure::HeroCount {
        expected: 4,
        seen: 5,
    };
    let bytes = image_jpeg();
    let path = retain_candidate_at(
        &root,
        PhotoCheckKind::ScenePlate,
        &bytes,
        ImageFormat::Jpeg,
        failure,
        "private generation prompt",
    )
    .unwrap();
    let line = record_retention(PhotoCheckKind::ScenePlate, failure, &Ok(path.clone()));
    assert!(
        line.contains(
            "PHOTO_CHECK kind=scene_plate result=fail reason=hero_count expected=4 seen=5"
        )
    );
    assert!(!line.contains("private generation prompt"));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap()).unwrap();
    assert_eq!(metadata["expected"], 4);
    assert_eq!(metadata["seen"], 5);
}

async fn fake_judgments(
    answers: Vec<Value>,
) -> (NativePlateJudge, tokio::sync::oneshot::Receiver<Value>) {
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
    let config = PlateJudgeConfig::loopback_fixture(address).unwrap();
    (
        NativePlateJudge::new(
            config,
            "test-token",
            Duration::from_secs(5),
            crate::gameplay::cost::test_meter(),
        )
        .unwrap(),
        request_receiver,
    )
}

async fn rejected(kind: PhotoCheckKind, answer: Value, expected: PhotoCheckFailure) {
    let (judge, _) = fake_judge(answer).await;
    let result = check_photo(
        &judge,
        kind,
        &image_jpeg(),
        ImageFormat::Jpeg,
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(result, PhotoCheckOutcome::Fail(expected));
}

#[tokio::test]
async fn rejects_contact_sheets_and_cutout_composites() {
    for (frame, expected) in [
        ("grid_or_collage", PhotoCheckFailure::GridOrCollage),
        ("cutout_composite", PhotoCheckFailure::CutoutComposite),
    ] {
        rejected(
            PhotoCheckKind::ScenePlate,
            answer(frame, "photograph", "none", "period", None),
            expected,
        )
        .await;
    }
}

#[tokio::test]
async fn rejects_painterly_and_rendered_media() {
    for medium in ["painting_or_illustration", "cgi_or_render"] {
        rejected(
            PhotoCheckKind::ScenePlate,
            answer("single_photograph", medium, "none", "period", None),
            PhotoCheckFailure::NotPhotograph,
        )
        .await;
    }
}

#[tokio::test]
async fn rejects_visible_text_and_modern_clothing() {
    rejected(
        PhotoCheckKind::ScenePlate,
        answer(
            "single_photograph",
            "photograph",
            "visible_text",
            "period",
            None,
        ),
        PhotoCheckFailure::VisibleText,
    )
    .await;
    rejected(
        PhotoCheckKind::ScenePlate,
        answer(
            "single_photograph",
            "photograph",
            "none",
            "modern_clothing",
            None,
        ),
        PhotoCheckFailure::ModernClothing,
    )
    .await;
}

#[tokio::test]
async fn rejects_people_in_an_empty_canonical_room() {
    let mut room_answer = answer("single_photograph", "photograph", "none", "period", None);
    room_answer["people_present"] = json!("some");
    rejected(
        PhotoCheckKind::CanonicalRoom,
        room_answer,
        PhotoCheckFailure::PeopleInEmptyRoom,
    )
    .await;
}

#[tokio::test]
async fn rejects_a_character_with_not_exactly_one_person() {
    rejected(
        PhotoCheckKind::CanonicalCharacter,
        answer(
            "single_photograph",
            "photograph",
            "none",
            "period",
            Some("not_one"),
        ),
        PhotoCheckFailure::CharacterCountNotOne,
    )
    .await;
}

#[tokio::test]
async fn rejects_a_canonical_character_answered_as_having_no_people() {
    rejected(
        PhotoCheckKind::CanonicalCharacter,
        answer(
            "single_photograph",
            "photograph",
            "none",
            "no_people",
            Some("not_one"),
        ),
        PhotoCheckFailure::CharacterHasNoPeople,
    )
    .await;
}

#[tokio::test]
async fn accepts_a_photographic_plate_and_requires_character_count_in_the_schema() {
    let (judge, _) = fake_judge(answer(
        "single_photograph",
        "photograph",
        "none",
        "period",
        None,
    ))
    .await;
    assert_eq!(
        check_photo(
            &judge,
            PhotoCheckKind::ScenePlate,
            &image_jpeg(),
            ImageFormat::Jpeg,
            Duration::from_secs(5),
        )
        .await
        .unwrap(),
        PhotoCheckOutcome::Pass
    );
}

#[tokio::test]
async fn rejects_a_modern_place_even_with_period_people() {
    let mut modern = answer("single_photograph", "photograph", "none", "no_people", None);
    modern["setting"] = json!("modern");
    modern["people_present"] = json!("none");
    rejected(
        PhotoCheckKind::CanonicalRoom,
        modern,
        PhotoCheckFailure::ModernSetting,
    )
    .await;
}

#[tokio::test]
async fn canonical_room_uses_explicit_people_presence_and_high_detail() {
    for (people_present, expected) in [
        (
            "some",
            PhotoCheckOutcome::Fail(PhotoCheckFailure::PeopleInEmptyRoom),
        ),
        ("none", PhotoCheckOutcome::Pass),
    ] {
        let mut room_answer = answer("single_photograph", "photograph", "none", "period", None);
        room_answer["people_present"] = json!(people_present);
        let (judge, request_receiver) = fake_judge(room_answer).await;
        assert_eq!(
            check_photo(
                &judge,
                PhotoCheckKind::CanonicalRoom,
                &image_jpeg(),
                ImageFormat::Jpeg,
                Duration::from_secs(5),
            )
            .await
            .unwrap(),
            expected
        );
        let request = request_receiver.await.unwrap();
        let content = request["messages"][0]["content"].as_array().unwrap();
        let instruction = content[0]["text"].as_str().unwrap();
        assert!(instruction.contains("clean, evenly lit establishing shot of an empty place"));
        assert!(instruction.contains("are not evidence of rendering"));
        assert!(instruction.contains("Require positive visible evidence of another medium"));
        assert!(instruction.contains("brushstrokes, drawn outlines or visibly painted textures"));
        assert!(instruction.contains("visibly synthetic geometry, plastic-like materials"));
        assert_eq!(content[2]["image_url"]["detail"], "high");
        let schema = &request["response_format"]["json_schema"]["schema"];
        assert_eq!(
            schema["properties"]["people_present"]["enum"],
            json!(["none", "some"])
        );
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("people_present"))
        );
        assert_eq!(schema["additionalProperties"], false);
    }
}

#[test]
fn photographic_correction_preserves_an_empty_rooms_people_count() {
    let correction = PhotoCheckFailure::NotPhotograph.correction();
    assert!(correction.contains("exactly zero people for an empty place"));
    assert!(correction.contains("Clean surfaces and even daylight are welcome"));
    assert!(!correction.contains("real people"));
}

#[tokio::test]
async fn saved_photo_check_calls_the_judge_once_with_the_files_picture_and_kind() {
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS16/saved-photo-check")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let bytes = image_jpeg();
    let path = directory.join("saved candidate.image");
    std::fs::write(&path, &bytes).unwrap();
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
        // A fixture accepts exactly one request and then closes its listener. An extra judge
        // invocation would fail the successful result; the captured body pins bytes and kind.
        let (saved_judge, saved_request) = fake_judge(reply.clone()).await;
        assert_eq!(
            super::super::Imagery::check_saved_photo(&saved_judge, kind.into(), &path)
                .await
                .unwrap(),
            PhotoCheckOutcome::Pass
        );
        let (direct_judge, direct_request) = fake_judge(reply).await;
        assert_eq!(
            check_photo(
                &direct_judge,
                kind,
                &bytes,
                ImageFormat::Jpeg,
                Duration::from_secs(5),
            )
            .await
            .unwrap(),
            PhotoCheckOutcome::Pass
        );
        assert_eq!(saved_request.await.unwrap(), direct_request.await.unwrap());
    }
}

#[tokio::test]
async fn saved_photo_check_reports_the_closed_reason_and_refuses_bad_files_before_judging() {
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS16/saved-photo-failure")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("candidate.image");
    std::fs::write(&path, image_jpeg()).unwrap();
    let (judge, _) = fake_judge(room_answer("none", "modern")).await;
    let outcome =
        super::super::Imagery::check_saved_photo(&judge, PhotoJudgeKind::CanonicalRoom, &path)
            .await
            .unwrap();
    assert_eq!(
        outcome,
        PhotoCheckOutcome::Fail(PhotoCheckFailure::ModernSetting)
    );
    assert_eq!(outcome.verdict(), "fail");
    assert_eq!(outcome.reason(), Some("modern_setting"));
    let (judge, mut requests) = fake_judge(room_answer("none", "period")).await;
    assert_eq!(
        super::super::Imagery::check_saved_photo(
            &judge,
            PhotoJudgeKind::CanonicalRoom,
            &directory.join("missing.image"),
        )
        .await,
        Err(PhotoCheckError::FileRead)
    );
    std::fs::write(&path, b"not an image").unwrap();
    assert_eq!(
        super::super::Imagery::check_saved_photo(&judge, PhotoJudgeKind::ScenePlate, &path).await,
        Err(PhotoCheckError::ImagePreparation)
    );
    let oversized = directory.join("oversized.image");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    assert_eq!(
        super::super::Imagery::check_saved_photo(&judge, PhotoJudgeKind::ScenePlate, &oversized)
            .await,
        Err(PhotoCheckError::FileTooLarge)
    );
    assert!(matches!(
        requests.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
}

#[test]
fn rejected_candidate_keeps_exact_bytes_reason_prompt_and_bounded_history() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS16/reject-retention-test")
        .join(std::process::id().to_string());
    let bytes = image_jpeg();
    let path = retain_candidate_at(
        &root,
        PhotoCheckKind::CanonicalRoom,
        &bytes,
        ImageFormat::Jpeg,
        PhotoCheckFailure::VisibleText,
        "The exact original room prompt",
    )
    .unwrap();
    let line = record_retention(
        PhotoCheckKind::CanonicalRoom,
        PhotoCheckFailure::VisibleText,
        &Ok(path.clone()),
    );
    assert!(line.contains("PHOTO_CHECK kind=canonical_room result=fail reason=visible_text"));
    assert!(line.contains(&format!("path={:?}", path.display().to_string())));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap()).unwrap();
    assert_eq!(metadata["reason"], "visible_text");
    assert_eq!(metadata["prompt"], "The exact original room prompt");
    assert_eq!(metadata["kind"], "canonical_room");
    for _ in 0..MAXIMUM_REJECTED_CANDIDATES + 3 {
        retain_candidate_at(
            &root,
            PhotoCheckKind::CanonicalRoom,
            &bytes,
            ImageFormat::Jpeg,
            PhotoCheckFailure::ModernSetting,
            "corrected room prompt",
        )
        .unwrap();
    }
    assert_eq!(
        std::fs::read_dir(root.join("photo-check-rejected"))
            .unwrap()
            .count(),
        MAXIMUM_REJECTED_CANDIDATES * 2
    );
}

fn room_answer(text: &str, setting: &str) -> Value {
    let mut value = answer("single_photograph", "photograph", text, "no_people", None);
    value["setting"] = json!(setting);
    value["people_present"] = json!("none");
    value
}

fn canonical_room_request() -> super::super::ImageRequest {
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
        references: vec![imagery::ReferenceImage::new(
            ImageFormat::Jpeg,
            bytes::Bytes::from(image_jpeg()),
        )],
        prompt: imagery::canonical_room(ContentRating::Restrained, &model, "test-street", "P1")
            .unwrap(),
        output: imagery::OutputSpec::BACKDROP,
    }
}

fn scene_plate_request(heroes: usize, edit: bool) -> super::super::ImageRequest {
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
            ReferenceImage::new(ImageFormat::Jpeg, image_jpeg().into()),
            ReferenceImage::new(ImageFormat::Jpeg, image_jpeg().into()),
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
                let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join(
                        "../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS16/scene-retry-pass",
                    )
                    .join(format!("{}-{heroes}-{edit}-{retry}", std::process::id()));
                let fixture = DirectionFixture::photographic(&root);
                let mut answers = Vec::new();
                if retry {
                    answers.push(scene_answer(heroes as u64 + 1));
                }
                answers.push(scene_answer(heroes as u64));
                let (judge, request_seen) = fake_judgments(answers).await;
                let imagery::Mode::On { engine, .. } = &fixture.imagery.inner.mode else {
                    panic!("enabled fixture");
                };
                assert!(engine.judge.set(judge).is_ok());
                let request = scene_plate_request(heroes, edit);
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
            }
        }
    }
}

#[tokio::test]
async fn persistent_scene_count_mismatch_keeps_the_fallback_and_never_publishes_rejected_bytes() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    for edit in [false, true] {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS16/scene-retry-bound")
            .join(format!("{}-{edit}", std::process::id()));
        let fixture = DirectionFixture::photographic(&root);
        let mut wrong = scene_answer(7);
        wrong["text"] = json!("visible_text");
        let (judge, _) = fake_judgments(vec![wrong.clone(), wrong]).await;
        let imagery::Mode::On { engine, .. } = &fixture.imagery.inner.mode else {
            panic!("enabled fixture");
        };
        assert!(engine.judge.set(judge).is_ok());
        let key = fixture.imagery.key_for(&scene_plate_request(6, edit));
        for _ in 0..2 {
            assert_eq!(
                fixture
                    .imagery
                    .generate("plate:test-room", scene_plate_request(6, edit))
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
                .peek(&scene_plate_request(6, edit))
                .await
                .is_none()
        );
        assert!(!engine.cached_now(key.as_str()));
    }
}

#[tokio::test]
async fn failing_then_passing_room_uses_the_judges_reason_in_the_second_generation_prompt() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS16/room-retry-pass")
        .join(std::process::id().to_string());
    let fixture = DirectionFixture::photographic(&root);
    let (judge, _) = fake_judgments(vec![
        room_answer("visible_text", "period"),
        room_answer("none", "period"),
    ])
    .await;
    let imagery::Mode::On { engine, .. } = &fixture.imagery.inner.mode else {
        panic!("enabled fixture");
    };
    assert!(engine.judge.set(judge).is_ok());
    let request = canonical_room_request();
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
}

#[tokio::test]
async fn persistently_rejected_room_stops_at_two_generations_until_readmission() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS37/fixture-cache/lanes/HS16/room-retry-bound")
        .join(std::process::id().to_string());
    let fixture = DirectionFixture::photographic(&root);
    let (judge, _) = fake_judgments(vec![
        room_answer("none", "modern"),
        room_answer("none", "modern"),
    ])
    .await;
    let imagery::Mode::On { engine, .. } = &fixture.imagery.inner.mode else {
        panic!("enabled fixture");
    };
    assert!(engine.judge.set(judge).is_ok());
    for _ in 0..2 {
        assert_eq!(
            fixture
                .imagery
                .generate("place:test-street", canonical_room_request())
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
}

#[tokio::test]
async fn scene_photo_rejects_unrecorded_drawn_weapons_with_a_typed_reason() {
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg().into());
    for recorded_draw in [false, true] {
        let mut value = scene_answer(6);
        value["weapon_custody"] = json!(if recorded_draw {
            "matches_custody"
        } else {
            "unexpected_drawn_weapon"
        });
        let (judge, request) = fake_judge(value).await;
        let prompt = if recorded_draw {
            "Vesper: sword drawn in right hand, left hand empty. Other heroes: both hands empty."
        } else {
            "Vesper: both hands empty, sword sheathed at hip. Other heroes: both hands empty."
        };
        let outcome = check_scene_photo_with_custody(
            &judge,
            &image_jpeg(),
            ImageFormat::Jpeg,
            SceneParty::new(6, 2).unwrap(),
            &sheet,
            prompt,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(
            outcome,
            if recorded_draw {
                PhotoCheckOutcome::Pass
            } else {
                PhotoCheckOutcome::Fail(PhotoCheckFailure::UnexpectedDrawnWeapon)
            }
        );
        let request = request.await.unwrap();
        let instruction = request["messages"][0]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert!(instruction.contains(prompt));
        assert!(instruction.contains("bare sword pointed down still counts as drawn"));
        assert_eq!(
            request["response_format"]["json_schema"]["schema"]["properties"]["weapon_custody"]["enum"],
            json!(["matches_custody", "unexpected_drawn_weapon"])
        );
    }
}

#[test]
fn scene_photo_custody_is_required_and_count_failures_keep_priority_over_text() {
    let party = SceneParty::new(6, 2).unwrap();
    for invalid in [Value::Null, json!("unknown"), json!(false)] {
        let mut value = scene_answer(6);
        value["weapon_custody"] = invalid;
        assert_eq!(
            parse_scene_photo_answer(&value, party),
            Err(PlateJudgeError::Output)
        );
    }
    let mut value = scene_answer(6);
    value.as_object_mut().unwrap().remove("weapon_custody");
    assert_eq!(
        parse_scene_photo_answer(&value, party),
        Err(PlateJudgeError::Output)
    );
    let mut value = scene_answer(7);
    value["text"] = json!("visible_text");
    value["weapon_custody"] = json!("unexpected_drawn_weapon");
    assert_eq!(
        parse_scene_photo_answer(&value, party),
        Ok(PhotoCheckOutcome::Fail(PhotoCheckFailure::HeroCount {
            expected: 6,
            seen: 7
        }))
    );
    value["hero_count"] = json!(6);
    assert_eq!(
        parse_scene_photo_answer(&value, party),
        Ok(PhotoCheckOutcome::Fail(
            PhotoCheckFailure::UnexpectedDrawnWeapon
        ))
    );
}

#[tokio::test]
async fn unexpected_drawn_weapon_uses_the_existing_plate_regeneration_gate() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    for edit in [false, true] {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS37/weapon-retry")
            .join(format!("{}-{edit}", std::process::id()));
        let fixture = DirectionFixture::photographic(&root);
        let mut wrong = scene_answer(6);
        wrong["weapon_custody"] = json!("unexpected_drawn_weapon");
        let (judge, _) = fake_judgments(vec![wrong, scene_answer(6)]).await;
        let imagery::Mode::On { engine, .. } = &fixture.imagery.inner.mode else {
            panic!("enabled fixture")
        };
        assert!(engine.judge.set(judge).is_ok());
        let key = fixture
            .imagery
            .generate("plate:weapon-custody", scene_plate_request(6, edit))
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
    }
}

#[tokio::test]
async fn scene_photo_uses_compact_hand_facts_within_the_judge_instruction_limit() {
    let custody = json!(
        (1..=9)
            .map(|ordinal| json!({
                "name": format!("Person {ordinal} {}", "N".repeat(32)),
                "hands": "left hand empty, right hand empty; sword sheathed at hip"
            }))
            .collect::<Vec<_>>()
    )
    .to_string();
    let (judge, request) = fake_judge(scene_answer(6)).await;
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg().into());
    assert_eq!(
        check_scene_photo_with_custody(
            &judge,
            &image_jpeg(),
            ImageFormat::Jpeg,
            SceneParty::new(6, 2).unwrap(),
            &sheet,
            &custody,
            Duration::from_secs(5)
        )
        .await
        .unwrap(),
        PhotoCheckOutcome::Pass
    );
    let request = request.await.unwrap();
    let instruction = request["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(instruction.chars().count() <= 6000);
    assert!(instruction.contains("Person 9"));
    assert!(instruction.contains("sword sheathed at hip"));
}

#[tokio::test]
async fn production_scene_gate_receives_compact_custody_for_fresh_and_edited_plates() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    let custody =
        "Vesper: sword drawn in right hand, left hand empty. Other heroes: both hands empty.";
    for edit in [false, true] {
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS37/production-custody")
            .join(format!("{}-{edit}", std::process::id()));
        let fixture = DirectionFixture::photographic(&cache);
        let (judge, seen) = fake_judge(scene_answer(6)).await;
        let imagery::Mode::On { engine, .. } = &fixture.imagery.inner.mode else {
            panic!("enabled")
        };
        assert!(engine.judge.set(judge).is_ok());
        let mut request = scene_plate_request(6, edit);
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
    }
}
#[tokio::test]
async fn photo_publication_gate_preserves_features_and_public_record_ids_without_an_extra_judgement()
 {
    let mut answer = scene_answer(2);
    answer["visible_features"] = json!([{
        "name":"fruit stand", "look":"Baskets of red apples under canvas.",
        "region":[600,200,900,700], "recorded_id":null
    }]);
    let (judge, sent) = fake_judge(answer).await;
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg().into());
    let (failure, features) = super::super::photo_failure(
        &judge,
        PhotoCheckKind::ScenePlate,
        &image_jpeg(),
        ImageFormat::Jpeg,
        "Photographic market scene.",
        Some((SceneParty::new(2, 2).unwrap(), &sheet)),
        r#"{"hands":[],"public_records":[{"id":"market-wenna","name":"Wenna"}]}"#,
    )
    .await;
    assert!(failure.is_none());
    assert_eq!(features.len(), 1);
    assert_eq!(features[0].name, "fruit stand");
    assert!(sent.await.unwrap().to_string().contains("market-wenna"));
}
#[tokio::test]
async fn photo_feature_public_catalog_keeps_quality_and_custody_within_the_instruction_limit() {
    let records=(0..256).map(|index|json!({"id":format!("market-item-{index}"),"name":format!("recorded object {index}")})).collect::<Vec<_>>();
    let custody =
        json!({"hands":"Pip: both hands empty; sword sheathed.","public_records":records})
            .to_string();
    let mut answer = scene_answer(6);
    answer["visible_features"] = json!([{"name":"recorded object 255","look":"An already recorded public object.",
        "region":[100,200,300,400],"recorded_id":"market-item-255"}]);
    let (judge, sent) = fake_judge(answer).await;
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg().into());
    let result = super::super::Imagery::read_scene_photo(
        &judge,
        &image_jpeg(),
        ImageFormat::Jpeg,
        SceneParty::new(6, 2).unwrap(),
        &sheet,
        &custody,
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(result.outcome, PhotoCheckOutcome::Pass);
    let request = sent.await.unwrap();
    let instruction = request["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(instruction.chars().count() <= 6000);
    assert!(instruction.contains("sword sheathed"));
    assert_eq!(
        result.visible_features[0].recorded_id.as_deref(),
        Some("market-item-255")
    );
    assert!(request["response_format"]["json_schema"]["schema"]["properties"]["visible_features"]["items"]["properties"]["recorded_id"]["enum"]
        .as_array().unwrap().contains(&json!("market-item-255")));
    assert!(request["response_format"]["json_schema"]["schema"]["properties"]["visible_features"]["description"]
        .as_str().unwrap().contains("market-item-255"));
}

#[tokio::test]
async fn photo_check_keeps_only_the_accepted_retry_features_with_the_cached_image_key() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS40/photo-evidence-cache")
        .join(std::process::id().to_string());
    let fixture = DirectionFixture::photographic(&cache);
    let mut accepted = scene_answer(6);
    accepted["visible_features"] = json!([{"name":"fruit stand","look":"Apples under canvas.","region":[600,200,900,700],"recorded_id":null}]);
    let mut rejected = accepted.clone();
    rejected["hero_count"] = json!(5);
    rejected["visible_features"][0]["name"] = json!("rejected fish cart");
    let (judge, sent) = fake_judgments(vec![rejected, accepted]).await;
    let imagery::Mode::On { engine, .. } = &fixture.imagery.inner.mode else {
        panic!("enabled imagery")
    };
    assert!(engine.judge.set(judge).is_ok());
    let mut request = scene_plate_request(6, false);
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
    assert_eq!(engine.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
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
}

#[tokio::test]
async fn photo_features_without_a_bound_place_do_not_break_legacy_plate_requests() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS40/photo-unbound-cache")
        .join(std::process::id().to_string());
    let fixture = DirectionFixture::photographic(&cache);
    let mut accepted = scene_answer(6);
    accepted["visible_features"] = json!([{"name":"fruit stand","look":"Apples under canvas.","region":[600,200,900,700],"recorded_id":null}]);
    let (judge, sent) = fake_judge(accepted).await;
    let imagery::Mode::On { engine, .. } = &fixture.imagery.inner.mode else {
        panic!("enabled imagery")
    };
    assert!(engine.judge.set(judge).is_ok());
    let mut request = scene_plate_request(6, false);
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
    assert_eq!(engine.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    sent.await.unwrap();
}
