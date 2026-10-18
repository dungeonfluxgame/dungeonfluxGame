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
    });
    if let Some(people_count) = people_count {
        answer["people_count"] = json!(people_count);
    }
    answer
}

async fn fake_judge(answer: Value) -> NativePlateJudge {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let body_length = loop {
            let mut chunk = [0_u8; 4096];
            let read = stream.read(&mut chunk).await.unwrap();
            assert_ne!(read, 0);
            request.extend_from_slice(&chunk[..read]);
            if let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
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
    });
    let config = PlateJudgeConfig::loopback_fixture(address).unwrap();
    NativePlateJudge::new(config, "test-token", Duration::from_secs(5)).unwrap()
}

async fn rejected(kind: PhotoCheckKind, answer: Value, expected: PhotoCheckFailure) {
    let judge = fake_judge(answer).await;
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
    rejected(
        PhotoCheckKind::CanonicalRoom,
        answer("single_photograph", "photograph", "none", "period", None),
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
    let judge = fake_judge(answer(
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
