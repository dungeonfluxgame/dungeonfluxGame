//! Legacy photo-check cases relocated to the actual private photographic adapter.
//! No public provider executor or consumer-side policy implementation is retained.
use crate::backends::analysis::{InspectionInput, PreparedInspection, photo, testing};
use crate::runtime::inspection_policy::{
    self, InspectionAssessment, InspectionError, InspectionVerdict, PhotoCheckFailure,
};
use crate::{
    InspectionKind,
    config::AnalysisPreparationLimits,
    request::{AnalysisRoute, PrivateImageText},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{io::Cursor, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
type PhotoCheckKind = InspectionKind;
type PhotoCheckOutcome = InspectionVerdict;
type ImageFormat = crate::backends::api::ImageFormat;
#[derive(Debug, Eq, PartialEq)]
enum PhotoCheckError {
    Judge(InspectionError),
}
struct FixtureJudge {
    route: AnalysisRoute,
}
struct ReferenceImage {
    bytes: Vec<u8>,
}
impl ReferenceImage {
    fn new(_format: ImageFormat, bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}
#[derive(Clone, Copy)]
struct SceneParty(u8);
impl SceneParty {
    fn new(heroes: usize, sheet_image: usize) -> Result<Self, InspectionError> {
        let heroes = u8::try_from(heroes)
            .ok()
            .filter(|count| (1..=6).contains(count))
            .ok_or(InspectionError::InvalidRequest)?;
        if sheet_image == 0 {
            return Err(InspectionError::InvalidRequest);
        }
        Ok(Self(heroes))
    }
}
struct ScenePhotoReading {
    outcome: InspectionVerdict,
    visible_features: Vec<inspection_policy::VisibleFeature>,
}
async fn fake_judge(answer: Value) -> (FixtureJudge, tokio::sync::oneshot::Receiver<Value>) {
    let (address, receiver) = fake_judgments_endpoint(vec![answer]).await;
    let mut route = testing::spec(InspectionKind::ScenePlate, false).route;
    route.endpoint =
        crate::ApiEndpoint::new(&format!("http://{address}/v1/chat/completions")).unwrap();
    route.model = crate::ModelName::new("gpt-5.5").unwrap();
    route.maximum_output_tokens = 3000;
    (FixtureJudge { route }, receiver)
}
fn scene_spec(
    kind: InspectionKind,
    party: Option<SceneParty>,
    custody: &str,
) -> crate::ImageInspection {
    let mut spec = testing::spec(kind, party.is_some());
    spec.expected_heroes = party.map(|party| party.0);
    spec.party_reference = party.map(|_| 1);
    spec.hand_custody = PrivateImageText::new(custody);
    spec
}
async fn run_photo(
    judge: &FixtureJudge,
    bytes: &[u8],
    kind: InspectionKind,
    party: Option<SceneParty>,
    sheet: Option<&ReferenceImage>,
    custody: &str,
    timeout: Duration,
) -> Result<InspectionAssessment, PhotoCheckError> {
    let mut spec = scene_spec(kind, party, custody);
    spec.route = judge.route.clone();
    let references = sheet
        .map(|sheet| vec![(testing::pin(&sheet.bytes), sheet.bytes.clone())])
        .unwrap_or_default();
    let response = PreparedInspection::prepare(
        InspectionInput {
            spec: &spec,
            candidate: bytes,
            references: &references,
            timeout,
            maximum_response_bytes: 256 * 1024,
            preparation: &AnalysisPreparationLimits::default(),
            max_decoded_bytes: 256 * 1024 * 1024,
        },
        "synthetic-key",
    )
    .unwrap()
    .execute()
    .await
    .unwrap();
    assert_eq!(response.status, 200);
    let value: Value = serde_json::from_slice(&response.body).unwrap();
    let answer = photo::answer(&value).map_err(PhotoCheckError::Judge)?;
    inspection_policy::interpret(answer, &spec).map_err(PhotoCheckError::Judge)
}
async fn check_photo(
    judge: &FixtureJudge,
    kind: InspectionKind,
    bytes: &[u8],
    _format: ImageFormat,
    timeout: Duration,
) -> Result<InspectionVerdict, PhotoCheckError> {
    Ok(run_photo(judge, bytes, kind, None, None, "", timeout)
        .await?
        .verdict)
}
async fn read_scene_photo(
    judge: &FixtureJudge,
    bytes: &[u8],
    _format: ImageFormat,
    party: SceneParty,
    sheet: &ReferenceImage,
    custody: &str,
    timeout: Duration,
) -> Result<ScenePhotoReading, PhotoCheckError> {
    let assessment = run_photo(
        judge,
        bytes,
        InspectionKind::ScenePlate,
        Some(party),
        Some(sheet),
        custody,
        timeout,
    )
    .await?;
    Ok(ScenePhotoReading {
        outcome: assessment.verdict,
        visible_features: assessment.visible_features,
    })
}
async fn check_scene_photo(
    judge: &FixtureJudge,
    bytes: &[u8],
    format: ImageFormat,
    party: SceneParty,
    sheet: &ReferenceImage,
    timeout: Duration,
) -> Result<InspectionVerdict, PhotoCheckError> {
    Ok(
        read_scene_photo(judge, bytes, format, party, sheet, "", timeout)
            .await?
            .outcome,
    )
}
async fn check_scene_photo_with_custody(
    judge: &FixtureJudge,
    bytes: &[u8],
    format: ImageFormat,
    party: SceneParty,
    sheet: &ReferenceImage,
    custody: &str,
    timeout: Duration,
) -> Result<InspectionVerdict, PhotoCheckError> {
    Ok(
        read_scene_photo(judge, bytes, format, party, sheet, custody, timeout)
            .await?
            .outcome,
    )
}
fn parse_scene_photo_answer(
    value: &Value,
    party: SceneParty,
) -> Result<InspectionVerdict, InspectionError> {
    inspection_policy::interpret(
        value.clone(),
        &scene_spec(InspectionKind::ScenePlate, Some(party), ""),
    )
    .map(|assessment| assessment.verdict)
}
async fn photo_failure(
    judge: &FixtureJudge,
    kind: InspectionKind,
    bytes: &[u8],
    format: ImageFormat,
    _prompt: &str,
    party: Option<(SceneParty, &ReferenceImage)>,
    custody: &str,
) -> (
    Option<PhotoCheckFailure>,
    Vec<inspection_policy::VisibleFeature>,
) {
    let Some((party, sheet)) = party else {
        panic!("scene fixture")
    };
    assert_eq!(kind, InspectionKind::ScenePlate);
    let reading = read_scene_photo(
        judge,
        bytes,
        format,
        party,
        sheet,
        custody,
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    let failure = match reading.outcome {
        InspectionVerdict::Pass => None,
        InspectionVerdict::Failed(failure) => Some(failure),
        InspectionVerdict::Unjudged(_) => panic!("unjudged fixture"),
    };
    (failure, reading.visible_features)
}
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
    assert_eq!(result, PhotoCheckOutcome::Failed(expected));
}

#[tokio::test]
async fn photo_check_reads_visible_features_in_the_same_quality_judgement() {
    let mut answer = scene_answer(6);
    answer["visible_features"] = json!([{
        "name": "fruit stand", "look": "A canvas stall with red apples in shallow baskets.",
        "region": [650, 220, 920, 600], "recorded_id": null,
    }]);
    let (judge, request) = fake_judge(answer).await;
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg());
    let reading = read_scene_photo(
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
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg());
    for (heroes, region, expected_error) in [
        (6, json!([600, 200, 500, 400]), true),
        (6, json!([0, 0, 1001, 1000]), true),
        (5, json!([0, 0, 1000, 1000]), false),
    ] {
        let mut answer = scene_answer(heroes);
        answer["visible_features"] = json!([{"name":"fruit stand", "look":"A distant canvas stall.",
            "region":region, "recorded_id":null}]);
        let (judge, _) = fake_judge(answer).await;
        let result = read_scene_photo(
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
                Err(PhotoCheckError::Judge(InspectionError::InvalidAnswer))
            ));
        } else {
            let reading = result.unwrap();
            assert!(matches!(
                reading.outcome,
                PhotoCheckOutcome::Failed(PhotoCheckFailure::HeroCount { .. })
            ));
            assert!(reading.visible_features.is_empty());
        }
    }
}

#[tokio::test]
async fn scene_photo_counts_against_the_sheet_and_requires_the_actual_count() {
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg());
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
                    PhotoCheckOutcome::Failed(PhotoCheckFailure::HeroCount {
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
        Ok(PhotoCheckOutcome::Failed(PhotoCheckFailure::HeroCount {
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
            &ReferenceImage::new(ImageFormat::Jpeg, image_jpeg()),
            Duration::from_secs(5),
        )
        .await
        .unwrap(),
        PhotoCheckOutcome::Failed(PhotoCheckFailure::HeroCount {
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
            Err(InspectionError::InvalidAnswer)
        );
    }
    let mut missing = scene_answer(2);
    missing.as_object_mut().unwrap().remove("hero_count");
    assert_eq!(
        parse_scene_photo_answer(&missing, party),
        Err(InspectionError::InvalidAnswer)
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
            Ok(PhotoCheckOutcome::Failed(failure))
        );
    }
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
            PhotoCheckOutcome::Failed(PhotoCheckFailure::PeopleInEmptyRoom),
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

#[tokio::test]
async fn scene_photo_rejects_unrecorded_drawn_weapons_with_a_typed_reason() {
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg());
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
                PhotoCheckOutcome::Failed(PhotoCheckFailure::UnexpectedDrawnWeapon)
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
            Err(InspectionError::InvalidAnswer)
        );
    }
    let mut value = scene_answer(6);
    value.as_object_mut().unwrap().remove("weapon_custody");
    assert_eq!(
        parse_scene_photo_answer(&value, party),
        Err(InspectionError::InvalidAnswer)
    );
    let mut value = scene_answer(7);
    value["text"] = json!("visible_text");
    value["weapon_custody"] = json!("unexpected_drawn_weapon");
    assert_eq!(
        parse_scene_photo_answer(&value, party),
        Ok(PhotoCheckOutcome::Failed(PhotoCheckFailure::HeroCount {
            expected: 6,
            seen: 7
        }))
    );
    value["hero_count"] = json!(6);
    assert_eq!(
        parse_scene_photo_answer(&value, party),
        Ok(PhotoCheckOutcome::Failed(
            PhotoCheckFailure::UnexpectedDrawnWeapon
        ))
    );
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
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg());
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
async fn photo_publication_gate_preserves_features_and_public_record_ids_without_an_extra_judgement()
 {
    let mut answer = scene_answer(2);
    answer["visible_features"] = json!([{
        "name":"fruit stand", "look":"Baskets of red apples under canvas.",
        "region":[600,200,900,700], "recorded_id":null
    }]);
    let (judge, sent) = fake_judge(answer).await;
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg());
    let (failure, features) = photo_failure(
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
    let sheet = ReferenceImage::new(ImageFormat::Jpeg, image_jpeg());
    let result = read_scene_photo(
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
async fn each_kind_matches_the_frozen_legacy_json_and_independent_opaque_jpeg80_bytes() {
    let frozen: Value = serde_json::from_str(include_str!("legacy_photo_requests.json")).unwrap();
    for (kind, label) in [
        (InspectionKind::ScenePlate, "scene_plate"),
        (InspectionKind::CanonicalRoom, "canonical_room"),
        (InspectionKind::CanonicalCharacter, "canonical_character"),
    ] {
        let mut answer = answer("single_photograph", "photograph", "none", "period", None);
        match kind {
            InspectionKind::CanonicalRoom => answer["people_present"] = json!("none"),
            InspectionKind::CanonicalCharacter => answer["people_count"] = json!("exactly_one"),
            InspectionKind::ScenePlate => {}
        }
        let bytes = image_jpeg();
        let (judge, request) = fake_judge(answer).await;
        assert_eq!(
            check_photo(
                &judge,
                kind,
                &bytes,
                ImageFormat::Jpeg,
                Duration::from_secs(5)
            )
            .await
            .unwrap(),
            InspectionVerdict::Pass
        );
        // This legacy fixture is an opaque2x2JPEG: matte is the identity and resizing is unnecessary.
        let rgb = image::load_from_memory(&bytes).unwrap().into_rgb8();
        assert_eq!(rgb.dimensions(), (2, 2));
        let mut encoded = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 80)
            .encode_image(&rgb)
            .unwrap();
        let mut expected = frozen[label].clone();
        expected["messages"][0]["content"][2]["image_url"]["url"] = json!(format!(
            "data:image/jpeg;base64,{}",
            STANDARD.encode(encoded)
        ));
        assert_eq!(request.await.unwrap(), expected);
    }
}
