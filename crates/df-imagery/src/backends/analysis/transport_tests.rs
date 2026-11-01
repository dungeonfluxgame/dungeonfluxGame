use super::super::testing;
use super::*;
use crate::InspectionKind;
use std::{
    io::{Read, Write},
    net::TcpListener,
};

#[test]
fn exact_body_facts_use_frozen_model_tokens_and_only_selected_sheet() {
    let candidate = testing::png(4, 2, [255, 0, 0, 128]);
    let sheet = testing::png(2, 2, [80, 120, 160, 255]);
    let references = vec![(testing::pin(&sheet), sheet)];
    let spec = testing::spec(InspectionKind::ScenePlate, true);
    let prepared = PreparedInspection::prepare(
        InspectionInput {
            spec: &spec,
            candidate: &candidate,
            references: &references,
            timeout: Duration::from_secs(2),
            maximum_response_bytes: 256 * 1024,
            preparation: &AnalysisPreparationLimits::default(),
            max_decoded_bytes: 256 * 1024 * 1024,
        },
        "synthetic-key",
    )
    .unwrap();
    assert_eq!(prepared.picture_count, 2);
    let contract = photo::contract(&spec).unwrap();
    assert_eq!(
        prepared.text_bytes,
        (contract.instruction.len() + contract.schema.to_string().len()) as u64
    );
    assert!(prepared.text_bytes < prepared.body_bytes);
    assert_eq!(prepared.body_bytes, prepared.request.body().len() as u64);
    assert_eq!(prepared.request.url(), spec.route.endpoint.as_str());
    let body: serde_json::Value = serde_json::from_slice(prepared.request.body()).unwrap();
    assert_eq!(body["model"], "operator/vision:v4");
    assert_eq!(body["max_completion_tokens"], 777);
    assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    let content = body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content[1]["text"], "Picture candidate-1:");
    assert_eq!(content[3]["text"], "Picture reference-sheet:");
    assert_eq!(
        content
            .iter()
            .filter(|part| part["type"] == "image_url")
            .count(),
        2
    );
    assert!(!format!("{prepared:?}").contains("synthetic-key"));
    assert!(!format!("{prepared:?}").contains("custom%20analysis"));
}

#[test]
fn picture_preparation_keeps_encoded_grey_matte_and_jpeg80() {
    let png = testing::png(4, 2, [255, 0, 0, 128]);
    let expected = image::RgbImage::from_pixel(4, 2, image::Rgb([191, 63, 63]));
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 80)
        .encode_image(&expected)
        .unwrap();
    assert_eq!(
        prepare_picture(
            &png,
            &AnalysisPreparationLimits::default(),
            256 * 1024 * 1024,
            png.len() as u64
        )
        .unwrap(),
        encoded
    );
    let wide = prepare_picture(
        &testing::png(2048, 8, [80, 120, 160, 255]),
        &AnalysisPreparationLimits::default(),
        256 * 1024 * 1024,
        4096,
    )
    .unwrap();
    assert_eq!(
        image::ImageReader::new(Cursor::new(wide))
            .with_guessed_format()
            .unwrap()
            .into_dimensions()
            .unwrap(),
        (1024, 4)
    );
}

#[test]
fn preparation_rejects_unbound_reference_zero_limits_and_invalid_key_without_sending() {
    let bytes = testing::png(2, 2, [80, 120, 160, 255]);
    let mut spec = testing::spec(InspectionKind::ScenePlate, true);
    let make = |spec: &ImageInspection, refs: &[(SourceRef, Vec<u8>)]| {
        PreparedInspection::prepare(
            InspectionInput {
                spec,
                candidate: &bytes,
                references: refs,
                timeout: Duration::from_secs(2),
                maximum_response_bytes: 1024,
                preparation: &AnalysisPreparationLimits::default(),
                max_decoded_bytes: 256 * 1024 * 1024,
            },
            "synthetic-key",
        )
    };
    assert!(matches!(make(&spec, &[]), Err(RuntimeError::InvalidRecord)));
    let mut pin = testing::pin(&bytes);
    pin.digest = crate::AssetDigest::from_bytes([0; 32]);
    assert!(matches!(
        make(&spec, &[(pin, bytes.clone())]),
        Err(RuntimeError::SourceDenied)
    ));
    spec.expected_heroes = None;
    spec.party_reference = None;
    spec.route.maximum_output_tokens = 0;
    assert!(matches!(
        make(&spec, &[]),
        Err(RuntimeError::InvalidConfiguration)
    ));
    spec.route.maximum_output_tokens = 777;
    assert!(matches!(
        PreparedInspection::prepare(
            InspectionInput {
                spec: &spec,
                candidate: &bytes,
                references: &[],
                timeout: Duration::from_secs(2),
                maximum_response_bytes: 1024,
                preparation: &AnalysisPreparationLimits::default(),
                max_decoded_bytes: 256 * 1024 * 1024,
            },
            "bad\nkey"
        ),
        Err(RuntimeError::InvalidConfiguration)
    ));
}

#[tokio::test]
async fn prepared_exchange_sends_once_and_returns_raw_status_and_usage_before_interpretation() {
    for status in [200, 503] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let mut spec = testing::spec(InspectionKind::CanonicalRoom, false);
        spec.route.endpoint =
            crate::ApiEndpoint::new(&format!("http://{address}/custom%20analysis/v2")).unwrap();
        let picture = testing::png(2, 2, [80, 120, 160, 255]);
        let prepared = PreparedInspection::prepare(
            InspectionInput {
                spec: &spec,
                candidate: &picture,
                references: &[],
                timeout: Duration::from_secs(3),
                maximum_response_bytes: 4096,
                preparation: &AnalysisPreparationLimits::default(),
                max_decoded_bytes: 256 * 1024 * 1024,
            },
            "synthetic-analysis-key",
        )
        .unwrap();
        assert!(
            matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock)
        );
        let raw = br#"{"usage":{"prompt_tokens":7,"completion_tokens":3},"choices":[]}"#;
        let server = std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(4);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < until);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        assert!(headers.starts_with("POST /custom%20analysis/v2 "));
                        assert!(headers.contains("Bearer synthetic-analysis-key"));
                        let body: serde_json::Value =
                            serde_json::from_slice(&request[end + 4..]).unwrap();
                        assert_eq!(body["max_completion_tokens"], 777);
                        break;
                    }
                }
            }
            write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",raw.len()).unwrap();
            stream.write_all(raw).unwrap();
            listener
        });
        let response = prepared.execute().await.unwrap();
        assert_eq!(response.status, status);
        assert_eq!(response.body, raw);
        let listener = server.join().unwrap();
        assert!(
            matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock)
        );
    }
}

#[tokio::test]
async fn an_expired_prepared_stage_does_not_reset_its_deadline_or_send() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut spec = testing::spec(InspectionKind::ScenePlate, false);
    spec.route.endpoint = crate::ApiEndpoint::new(&format!(
        "http://{}/inspection",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let picture = testing::png(2, 2, [80, 120, 160, 255]);
    let mut prepared = PreparedInspection::prepare(
        InspectionInput {
            spec: &spec,
            candidate: &picture,
            references: &[],
            timeout: Duration::from_secs(3),
            maximum_response_bytes: 1024,
            preparation: &AnalysisPreparationLimits::default(),
            max_decoded_bytes: 256 * 1024 * 1024,
        },
        "synthetic-key",
    )
    .unwrap();
    prepared.expires = Instant::now();
    assert!(matches!(
        prepared.execute().await,
        Err(RuntimeError::Expired)
    ));
    assert!(matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
}

#[test]
fn configured_limits_replace_static_pixel_picture_and_request_caps() {
    assert!(picture_budget(6000, 4000, 20_000_000, 1_000_000_000, 0).is_err());
    assert!(picture_budget(6000, 4000, 24_000_000, 1_000_000_000, 0).is_ok());
    assert!(picture_budget(6000, 4000, 24_000_000, 1000, 0).is_err());
    assert!(picture_budget(0, 4, 24_000_000, u64::MAX, 0).is_err());
    let bytes = testing::png(2, 2, [80, 120, 160, 255]);
    let spec = testing::spec(InspectionKind::CanonicalRoom, false);
    for limits in [
        AnalysisPreparationLimits {
            max_pixels: 3,
            ..Default::default()
        },
        AnalysisPreparationLimits {
            max_picture_bytes: 1,
            ..Default::default()
        },
        AnalysisPreparationLimits {
            max_request_bytes: 1,
            ..Default::default()
        },
    ] {
        assert!(
            PreparedInspection::prepare(
                InspectionInput {
                    spec: &spec,
                    candidate: &bytes,
                    references: &[],
                    timeout: Duration::from_secs(2),
                    maximum_response_bytes: 1024,
                    preparation: &limits,
                    max_decoded_bytes: 256 * 1024 * 1024,
                },
                "synthetic-key"
            )
            .is_err()
        );
    }
    assert!(
        prepare_picture(
            &bytes,
            &AnalysisPreparationLimits::default(),
            1,
            bytes.len() as u64
        )
        .is_err()
    );
}
