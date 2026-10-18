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
        NativePlateJudge::new(config, "test-token", Duration::from_secs(5)).unwrap(),
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
        .join("../../artifacts/tmp/lanes/J1/saved-photo-check")
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
        .join("../../artifacts/tmp/lanes/J1/saved-photo-failure")
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
        .join("../../artifacts/tmp/lanes/J1/reject-retention-test")
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

#[tokio::test]
async fn failing_then_passing_room_uses_the_judges_reason_in_the_second_generation_prompt() {
    use crate::gameplay::imagery::{self, tests::DirectionFixture};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/J1/room-retry-pass")
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
        .join("../../artifacts/tmp/lanes/J1/room-retry-bound")
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
