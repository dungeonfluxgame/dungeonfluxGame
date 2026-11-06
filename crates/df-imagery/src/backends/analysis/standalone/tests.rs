use super::super::testing;
use super::*;
use crate::{
    config::{AnalysisPreparationLimits, SegmentationProvider, SegmentationRoute},
    request::{AnalysisRoute, PrivateImageText},
    runtime::RuntimeError,
    service::{analysis_types::*, types::OperationHandle},
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};
const MEMORY: u64 = 256 * 1024 * 1024;
fn route(endpoint: &str) -> AnalysisRoute {
    AnalysisRoute {
        account: crate::AccountHandle::new("standalone-fixture").unwrap(),
        model: crate::ModelName::new("operator/vision:v7").unwrap(),
        endpoint: crate::ApiEndpoint::new(endpoint).unwrap(),
        maximum_output_tokens: 987,
    }
}
fn head_operation(bytes: &[u8]) -> ImageAnalysisOperation {
    ImageAnalysisOperation::HeadLocations(HeadLocationsSpec {
        source: testing::pin(bytes),
        face: None,
        targets: vec![AnalysisTarget {
            id: PrivateImageText::new("hero-1"),
            description: PrivateImageText::new("red-bearded dwarf"),
        }],
        contract_revision: 1,
    })
}
fn input<'a>(
    operation: &'a ImageAnalysisOperation,
    bytes: &'a [u8],
    route: &'a AnalysisRoute,
    limits: &'a AnalysisPreparationLimits,
) -> StandaloneChatInput<'a> {
    StandaloneChatInput {
        operation,
        source: bytes,
        face: None,
        references: &[],
        segmentation: None,
        locations: None,
        identity: None,
        route,
        preparation: limits,
        max_decoded_bytes: MEMORY,
        timeout: Duration::from_secs(2),
        maximum_response_bytes: 4096,
    }
}
fn handle() -> OperationHandle {
    OperationHandle {
        owner: crate::ImageOwner::Draft {
            account: df_types::AccountId::from_bytes(&[4; 16]).unwrap(),
        },
        operation: df_types::OperationId::from_bytes(&[5; 16]).unwrap(),
    }
}
fn segmentation(bytes: &[u8]) -> ImageAnalysisResult {
    fn set(
        prompt: SegmentationPrompt,
        left: u32,
        right: u32,
        top: u32,
        bottom: u32,
    ) -> AnalysisMaskSet {
        AnalysisMaskSet {
            prompt,
            width_px: 200,
            height_px: 100,
            instances: vec![AnalysisMask {
                index: 0,
                score: AnalysisUnit::new(0.95).unwrap(),
                runs: (top..bottom)
                    .map(|row| AnalysisMaskRun {
                        start: row * 200 + left,
                        length: right - left,
                    })
                    .collect(),
            }],
        }
    }
    ImageAnalysisResult::Segmentation {
        source: testing::pin(bytes),
        contract_revision: 1,
        masks: vec![
            set(SegmentationPrompt::Person, 20, 80, 20, 100),
            set(SegmentationPrompt::Head, 35, 60, 20, 40),
        ],
    }
}
pub(super) fn server(
    listener: TcpListener,
    responses: Vec<(u16, Vec<u8>)>,
) -> std::thread::JoinHandle<(TcpListener, Vec<Vec<u8>>)> {
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(4);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            // Accepted sockets can inherit the listener's nonblocking state on Windows.
            stream.set_nonblocking(false).unwrap();
            let read_deadline = Instant::now() + Duration::from_secs(2);
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let remaining = read_deadline
                    .checked_duration_since(Instant::now())
                    .filter(|remaining| !remaining.is_zero())
                    .expect("bounded fixture request read expired");
                stream.set_read_timeout(Some(remaining)).unwrap();
                let count = match stream.read(&mut buffer) {
                    Ok(count) => count,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        assert!(
                            Instant::now() < read_deadline,
                            "fixture request body did not arrive"
                        );
                        continue;
                    }
                    Err(error) => panic!("fixture request read failed: {error}"),
                };
                assert!(count > 0, "fixture request ended before its complete body");
                request.extend_from_slice(&buffer[..count]);
                assert!(
                    request.len() <= 8 * 1024 * 1024,
                    "fixture request exceeded its byte bound"
                );
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers.lines().find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    });
                    let length = length.unwrap_or_else(|| {
                        assert!(
                            headers.starts_with("GET "),
                            "fixture POST omitted Content-Length"
                        );
                        0
                    });
                    let total = end
                        .checked_add(4)
                        .and_then(|n| n.checked_add(length))
                        .filter(|total| *total <= 8 * 1024 * 1024)
                        .expect("fixture declared request exceeded its byte bound");
                    assert!(
                        request.len() <= total,
                        "fixture received bytes beyond Content-Length"
                    );
                    if request.len() == total {
                        break;
                    }
                }
            }
            requests.push(request);
            write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
            stream.write_all(&body).unwrap();
        }
        (listener, requests)
    })
}
#[test]
fn head_policy_retains_closed_target_filtering_plausibility_and_legacy_question() {
    let bytes = testing::png(2, 2, [255, 0, 0, 128]);
    let operation = head_operation(&bytes);
    let ImageAnalysisOperation::HeadLocations(spec) = &operation else {
        panic!()
    };
    let body = heads::request_body(
        b"jpeg",
        None,
        &spec.targets,
        &route("http://127.0.0.1:9/chat"),
    )
    .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["model"], "operator/vision:v7");
    assert_eq!(body["max_completion_tokens"], 987);
    assert!(
        body["messages"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("hero-1: red-bearded dwarf")
    );
    let response = json!({"choices":[{"message":{"content":"```json\n{\"heads\":[{\"id\":\"hero-1\",\"left\":0.1,\"top\":0.1,\"right\":0.2,\"bottom\":0.3},{\"id\":\"foreign\",\"left\":0.1,\"top\":0.1,\"right\":0.2,\"bottom\":0.3},{\"id\":\"hero-1\",\"left\":0.4,\"top\":0.1,\"right\":0.5,\"bottom\":0.3}]}\n```"}}]});
    let ImageAnalysisResult::HeadLocations { heads, .. } =
        interpret_chat(&response, &operation, None, None, None).unwrap()
    else {
        panic!()
    };
    assert_eq!(heads.len(), 1);
    assert_eq!(heads[0].head.coordinates(), [0.1, 0.1, 0.2, 0.3]);
    assert!(
        heads::parse_heads(
            r#"{"heads":[{"id":"hero-1","left":0.1,"top":0.1,"right":0.9,"bottom":0.3}]}"#,
            &spec.targets
        )
        .is_err()
    );
}
#[test]
fn staging_questions_and_conflicting_or_incomplete_choices_keep_legacy_rules() {
    let identity = staging::identity_request(2, 2).unwrap();
    assert_eq!(identity.schema_name, "plate-identity");
    assert_eq!(
        identity.schema["properties"]["matches"]["items"]["properties"]["candidate"]["enum"],
        json!([0, 1, 2])
    );
    let read=staging::read_identity(&json!({"matches":[{"reference":"ref-1","candidate":1},{"reference":"ref-2","candidate":1}]}),2,2).unwrap();
    assert_eq!(read.assignments, vec![None, None]);
    assert!(
        staging::read_identity(
            &json!({"matches":[{"reference":"ref-1","candidate":1}]}),
            2,
            2
        )
        .is_err()
    );
    let facing = staging::facing_request(&[2, 7]).unwrap();
    assert_eq!(facing.schema_name, "plate-facing");
    assert_eq!(
        facing.schema["properties"]["facings"]["items"]["properties"]["candidate"]["enum"],
        json!([2, 7])
    );
    assert_eq!(staging::read_facing(&json!({"facings":[{"candidate":7,"facing":"unclear"},{"candidate":2,"facing":"left"}]}),&[2,7]).unwrap(),vec![Some(staging::Facing::Left),None]);
    assert!(
        staging::read_facing(
            &json!({"facings":[{"candidate":2,"facing":"left"},{"candidate":2,"facing":"right"}]}),
            &[2, 7]
        )
        .is_err()
    );
}
#[test]
fn locator_complete_identity_is_local_and_foreign_source_dependency_is_refused() {
    let bytes = testing::png(200, 100, [20, 40, 60, 255]);
    let masks = segmentation(&bytes);
    let reference = testing::pin(&bytes);
    let operation = ImageAnalysisOperation::StagingIdentity(StagingIdentitySpec {
        source: reference.clone(),
        segmentation: handle(),
        locations: Some(handle()),
        references: vec![AnalysisReference {
            id: PrivateImageText::new("hero-1"),
            source: reference.clone(),
        }],
        contract_revision: 1,
    });
    let locations = ImageAnalysisResult::HeadLocations {
        source: reference,
        contract_revision: 1,
        heads: vec![AnalysisLocatedHead {
            id: PrivateImageText::new("hero-1"),
            head: NormalizedImageBox::new(0.175, 0.2, 0.3, 0.4).unwrap(),
        }],
    };
    let route = route("http://127.0.0.1:9/chat");
    let limits = AnalysisPreparationLimits::default();
    let mut request = input(&operation, &bytes, &route, &limits);
    request.segmentation = Some(&masks);
    request.locations = Some(&locations);
    let Some(ImageAnalysisResult::StagingIdentity { matches, .. }) =
        prepare_local_staging(&request).unwrap()
    else {
        panic!()
    };
    assert_eq!(matches[0].candidate, Some(1));
    let mut foreign = locations.clone();
    if let ImageAnalysisResult::HeadLocations { source, .. } = &mut foreign {
        source.digest = crate::AssetDigest::from_bytes([0; 32]);
    }
    request.locations = Some(&foreign);
    assert!(matches!(
        prepare_local_staging(&request),
        Err(RuntimeError::SourceDenied)
    ));
}
#[test]
fn legacy_matte_and_sam_alpha_drop_remain_distinct_jpeg_recipes() {
    let bytes = testing::png(4, 2, [255, 0, 0, 128]);
    let limits = AnalysisPreparationLimits::default();
    let vision = pictures::vision(&bytes, 1024, &limits, MEMORY, bytes.len() as u64).unwrap();
    let (sam, width, height) =
        pictures::plate(&bytes, &limits, MEMORY, bytes.len() as u64).unwrap();
    assert_eq!((width, height), (4, 2));
    for (actual, pixel, quality) in [(vision, [191, 63, 63], 80), (sam, [255, 0, 0], 90)] {
        let mut expected = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut expected, quality)
            .encode_image(&image::RgbImage::from_pixel(4, 2, image::Rgb(pixel)))
            .unwrap();
        assert_eq!(actual, expected);
    }
    assert!(pictures::plate(&bytes, &limits, 1, bytes.len() as u64).is_err());
}
#[test]
fn sam_rle_interpretation_preserves_one_based_runs_and_rejects_malformed_metadata() {
    let value =
        json!({"rle":["13 3 24 3",""],"scores":[0.9,0.8],"metadata":[{"index":7},{"index":8}]});
    let masks = parse_segmentation(&value, SegmentationPrompt::Head, 10, 5, 32).unwrap();
    assert_eq!(masks.instances.len(), 1);
    assert_eq!(masks.instances[0].index, 7);
    assert_eq!(
        masks.instances[0].runs[0],
        AnalysisMaskRun {
            start: 12,
            length: 3
        }
    );
    for bad in [
        json!({"rle":["0 3"],"scores":[0.9]}),
        json!({"rle":["50 3"],"scores":[0.9]}),
        json!({"rle":["1 3"],"scores":[]}),
        json!({"rle":["1 3"],"scores":[0.9],"metadata":[{"index":u64::MAX}]}),
    ] {
        assert!(parse_segmentation(&bad, SegmentationPrompt::Person, 10, 5, 32).is_err());
    }
}
#[tokio::test]
async fn prepared_chat_sends_once_with_frozen_route_and_returns_raw_status_usage() {
    for status in [200, 503] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let route = route(&format!(
            "http://{}/custom%20chat",
            listener.local_addr().unwrap()
        ));
        let bytes = testing::png(4, 2, [20, 40, 60, 255]);
        let operation = head_operation(&bytes);
        let limits = AnalysisPreparationLimits::default();
        let prepared = PreparedStandaloneChat::prepare(
            input(&operation, &bytes, &route, &limits),
            "synthetic-key",
        )
        .unwrap();
        assert_eq!(prepared.picture_count, 1);
        assert!(prepared.text_bytes < prepared.body_bytes);
        assert!(!format!("{prepared:?}").contains("custom%20chat"));
        assert!(
            matches!(listener.accept(),Err(error)if error.kind()==std::io::ErrorKind::WouldBlock)
        );
        let raw = br#"{"usage":{"prompt_tokens":11,"completion_tokens":3},"choices":[]}"#.to_vec();
        let fixture = server(listener, vec![(status, raw.clone())]);
        let response = prepared.execute().await.unwrap();
        assert_eq!(response.status, status);
        assert_eq!(response.body, raw);
        let (listener, requests) = fixture.join().unwrap();
        assert_eq!(requests.len(), 1);
        let request = String::from_utf8_lossy(&requests[0]);
        assert!(request.starts_with("POST /custom%20chat "));
        assert!(request.contains("operator/vision:v7"));
        assert!(request.contains("\"max_completion_tokens\":987"));
        assert!(
            matches!(listener.accept(),Err(error)if error.kind()==std::io::ErrorKind::WouldBlock)
        );
    }
}
#[tokio::test]
async fn sam_checkpoint_resume_is_get_only_confined_and_single_submit() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/custom%20base", listener.local_addr().unwrap());
    let route = SegmentationRoute {
        provider: SegmentationProvider::Fal,
        account: crate::AccountHandle::new("sam-fixture").unwrap(),
        model: crate::ModelName::new("fal-ai/custom-sam:v7/image-rle").unwrap(),
        endpoint: crate::ApiEndpoint::new(&base).unwrap(),
    };
    let bytes = testing::png(4, 2, [20, 40, 60, 255]);
    let pin = testing::pin(&bytes);
    let limits = AnalysisPreparationLimits {
        max_picture_bytes: 3 * 1024 * 1024,
        max_request_bytes: 6 * 1024 * 1024,
        ..Default::default()
    };
    let mut prepared = PreparedSegmentation::prepare(
        SegmentationInput {
            source: &bytes,
            source_pin: &pin,
            prompt: SegmentationPrompt::Person,
            preparation_recipe: SegmentationPreparation::ScenePlate,
            maximum_masks: 32,
            route: &route,
            preparation: &limits,
            max_decoded_bytes: MEMORY,
            timeout: Duration::from_secs(3),
            maximum_response_bytes: 4096,
        },
        "synthetic-key",
    )
    .unwrap();
    let submit = json!({"request_id":"request-1","status_url":format!("{base}/fal-ai/custom-sam:v7/requests/request-1/status"),"response_url":format!("{base}/fal-ai/custom-sam:v7/requests/request-1")});
    let terminal = br#"{"rle":["1 2"],"scores":[0.95]}"#.to_vec();
    let fixture = server(
        listener,
        vec![
            (202, submit.to_string().into_bytes()),
            (200, br#"{"status":"COMPLETED"}"#.to_vec()),
            (200, terminal.clone()),
        ],
    );
    let submitted = prepared.submit().await.unwrap();
    assert!(matches!(
        prepared.submit().await,
        Err(RuntimeError::InvalidRecord)
    ));
    let checkpoint = prepared.checkpoint(&submitted).unwrap();
    let encoded = checkpoint.encode();
    let checkpoint = SegmentationCheckpoint::decode(&encoded).unwrap();
    let response = prepared.resume(&checkpoint).await.unwrap();
    assert_eq!(response.body, terminal);
    let (listener, requests) = fixture.join().unwrap();
    let lines = requests
        .iter()
        .map(|request| {
            String::from_utf8_lossy(request)
                .lines()
                .next()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert!(lines[0].starts_with("POST /custom%20base/fal-ai/custom-sam:v7/image-rle "));
    assert!(
        lines[1].starts_with("GET /custom%20base/fal-ai/custom-sam:v7/requests/request-1/status ")
    );
    assert!(lines[2].starts_with("GET /custom%20base/fal-ai/custom-sam:v7/requests/request-1 "));
    for hostile in [
        format!("{base}/fal-ai/custom-sam:v7/requests/foreign/status"),
        format!("{base}/fal-ai/custom-sam:v7/requests/request-1/status#fragment"),
        format!("{base}/fal-ai/foreign/requests/request-1/status"),
        "http://127.0.0.1:9/custom%20base/fal-ai/custom-sam:v7/requests/request-1/status"
            .to_owned(),
    ] {
        let mut value: Value = serde_json::from_str(&encoded).unwrap();
        value["status_url"] = json!(hostile);
        assert!(SegmentationCheckpoint::decode(&value.to_string()).is_err());
    }
    assert!(matches!(listener.accept(),Err(error)if error.kind()==std::io::ErrorKind::WouldBlock));
}

#[test]
fn unsupported_shared_transport_limits_refuse_before_any_egress_without_clamping() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let route = route(&format!("http://{}/chat", listener.local_addr().unwrap()));
    let bytes = testing::png(2, 2, [20, 40, 60, 255]);
    let operation = head_operation(&bytes);
    let limits = AnalysisPreparationLimits {
        max_request_bytes: df_providers::MAXIMUM_BODY_BYTES as u64 + 1,
        ..Default::default()
    };
    assert!(matches!(
        PreparedStandaloneChat::prepare(
            input(&operation, &bytes, &route, &limits),
            "synthetic-key"
        ),
        Err(RuntimeError::UnsupportedCapability)
    ));
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[test]
fn figure_and_scene_plate_sam_preparation_are_explicit_distinct_recipes() {
    let bytes = testing::png(2000, 4, [255, 0, 0, 128]);
    let limits = AnalysisPreparationLimits {
        max_picture_bytes: 3 * 1024 * 1024,
        ..Default::default()
    };
    let (figure, fw, fh) = pictures::figure(&bytes, &limits, MEMORY, bytes.len() as u64).unwrap();
    let (scene, sw, sh) = pictures::plate(&bytes, &limits, MEMORY, bytes.len() as u64).unwrap();
    assert_eq!((fw, fh), (1536, 3));
    assert_eq!((sw, sh), (2000, 4));
    let pixel = |jpeg: Vec<u8>| {
        image::load_from_memory(&jpeg)
            .unwrap()
            .to_rgb8()
            .get_pixel(0, 0)
            .0
    };
    assert!(pixel(figure)[1] > 40);
    assert!(pixel(scene)[1] < 10);
}

#[tokio::test]
async fn standalone_photo_delegates_the_existing_single_inspection_without_generation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let bytes = testing::png(4, 2, [20, 40, 60, 255]);
    let mut inspection = testing::spec(crate::InspectionKind::CanonicalRoom, false);
    inspection.route.endpoint =
        crate::ApiEndpoint::new(&format!("http://{address}/inspection")).unwrap();
    inspection.maximum_corrections = 0;
    let spec = PhotoInspectionSpec {
        source: testing::pin(&bytes),
        references: Vec::new(),
        inspection,
    };
    let limits = AnalysisPreparationLimits::default();
    let prepared = PreparedPhotoInspection::prepare(
        PhotoInspectionInput {
            spec: &spec,
            source: &bytes,
            references: &[],
            preparation: &limits,
            max_decoded_bytes: MEMORY,
            timeout: Duration::from_secs(2),
            maximum_response_bytes: 4096,
        },
        "synthetic-key",
    )
    .unwrap();
    assert_eq!(prepared.picture_count, 1);
    assert!(prepared.body_bytes > prepared.text_bytes);
    let answer = json!({"frame":"single_photograph","medium":"photograph","text":"none","dress":"no_people","setting":"period","people_present":"none"});
    let raw = json!({"usage":{"prompt_tokens":11,"completion_tokens":3},"choices":[{"message":{"content":answer.to_string()}}]});
    let fixture = server(listener, vec![(200, raw.to_string().into_bytes())]);
    let response = prepared.execute().await.unwrap();
    let observed: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(observed, raw);
    let assessment = interpret_photo(&observed, &spec).unwrap();
    assert_eq!(assessment.verdict, PhotoInspectionVerdict::Pass);
    assert!(assessment.visible_features.is_empty());
    let (listener, requests) = fixture.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(String::from_utf8_lossy(&requests[0]).starts_with("POST /inspection "));
    let end = requests[0]
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap();
    let body: Value = serde_json::from_slice(&requests[0][end + 4..]).unwrap();
    assert_eq!(
        body["response_format"]["json_schema"]["name"],
        "photo-quality"
    );
    assert_eq!(body["max_completion_tokens"], 777);
    assert!(body.get("prompt").is_none());
    assert!(matches!(listener.accept(),Err(error)if error.kind()==std::io::ErrorKind::WouldBlock));
}

#[test]
fn legacy_staging_question_capacity_skips_paid_call_and_keeps_unknown_identities() {
    let bytes = testing::png(200, 100, [20, 40, 60, 255]);
    let pin = testing::pin(&bytes);
    let people = AnalysisMaskSet {
        prompt: SegmentationPrompt::Person,
        width_px: 200,
        height_px: 100,
        instances: (0..17)
            .map(|index| AnalysisMask {
                index,
                score: AnalysisUnit::new(0.95).unwrap(),
                runs: vec![AnalysisMaskRun {
                    start: 20 * 200 + index * 5,
                    length: 3,
                }],
            })
            .collect(),
    };
    let masks = ImageAnalysisResult::Segmentation {
        source: pin.clone(),
        contract_revision: 1,
        masks: vec![
            people,
            AnalysisMaskSet {
                prompt: SegmentationPrompt::Head,
                width_px: 200,
                height_px: 100,
                instances: Vec::new(),
            },
        ],
    };
    let operation = ImageAnalysisOperation::StagingIdentity(StagingIdentitySpec {
        source: pin.clone(),
        segmentation: handle(),
        locations: None,
        references: vec![AnalysisReference {
            id: PrivateImageText::new("hero-1"),
            source: pin.clone(),
        }],
        contract_revision: 1,
    });
    let Some(identity @ ImageAnalysisResult::StagingIdentity { .. }) =
        prepare_local_analysis(&operation, Some(&masks), None, None).unwrap()
    else {
        panic!()
    };
    if let ImageAnalysisResult::StagingIdentity { matches, .. } = &identity {
        assert_eq!(matches[0].candidate, None);
    }
    let facing = ImageAnalysisOperation::StagingFacing(StagingFacingSpec {
        source: pin,
        segmentation: handle(),
        identity: handle(),
        contract_revision: 1,
    });
    let Some(ImageAnalysisResult::StagingFacing { facings, .. }) =
        prepare_local_analysis(&facing, Some(&masks), None, Some(&identity)).unwrap()
    else {
        panic!()
    };
    assert!(facings.is_empty());
}

#[test]
fn tall_figure_recipe_refuses_unrepresentable_masks_before_any_supplier_send() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let route = SegmentationRoute {
        provider: SegmentationProvider::Fal,
        account: crate::AccountHandle::new("tall-mask-fixture").unwrap(),
        model: crate::ModelName::new("fal-ai/sam-3-1/image-rle").unwrap(),
        endpoint: crate::ApiEndpoint::new(&format!("http://{}", listener.local_addr().unwrap()))
            .unwrap(),
    };
    let limits = AnalysisPreparationLimits {
        max_picture_bytes: 3 * 1024 * 1024,
        max_request_bytes: 6 * 1024 * 1024,
        ..Default::default()
    };
    for (height, recipe, supported) in [
        (8192, SegmentationPreparation::FigureMask, true),
        (8193, SegmentationPreparation::FigureMask, false),
        (8193, SegmentationPreparation::ScenePlate, true),
    ] {
        let bytes = testing::png(2, height, [20, 40, 60, 255]);
        let pin = testing::pin(&bytes);
        let prepared = PreparedSegmentation::prepare(
            SegmentationInput {
                source: &bytes,
                source_pin: &pin,
                prompt: SegmentationPrompt::Person,
                preparation_recipe: recipe,
                maximum_masks: 8,
                route: &route,
                preparation: &limits,
                max_decoded_bytes: MEMORY,
                timeout: Duration::from_secs(3),
                maximum_response_bytes: 4096,
            },
            "synthetic-key",
        );
        if supported {
            assert!(prepared.is_ok());
        } else {
            assert!(matches!(prepared, Err(RuntimeError::UnsupportedCapability)));
        }
    }
    assert!(matches!(listener.accept(),Err(error)if error.kind()==std::io::ErrorKind::WouldBlock));
}
