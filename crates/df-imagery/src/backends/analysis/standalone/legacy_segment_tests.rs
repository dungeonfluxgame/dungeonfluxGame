//! Queue and malformed-result cases relocated from the retired public SAM executor.
use super::{super::testing, tests::server, *};
use crate::{
    config::{AnalysisPreparationLimits, SegmentationProvider, SegmentationRoute},
    service::analysis_types::*,
};
use serde_json::{Value, json};
use std::{
    net::{SocketAddr, TcpListener},
    time::Duration,
};
const MEMORY: u64 = 256 * 1024 * 1024;
fn prepare(
    address: SocketAddr,
    model: &str,
    maximum_masks: u32,
    timeout: Duration,
) -> Result<PreparedSegmentation, RuntimeError> {
    let source = testing::png(40, 20, [20, 40, 60, 255]);
    let pin = testing::pin(&source);
    let route = SegmentationRoute {
        provider: SegmentationProvider::Fal,
        account: crate::AccountHandle::new("legacy-sam-fixture").unwrap(),
        model: crate::ModelName::new(model).unwrap(),
        endpoint: crate::ApiEndpoint::new(&format!("http://{address}")).unwrap(),
    };
    PreparedSegmentation::prepare(
        SegmentationInput {
            source: &source,
            source_pin: &pin,
            prompt: SegmentationPrompt::Person,
            preparation_recipe: SegmentationPreparation::ScenePlate,
            maximum_masks,
            route: &route,
            preparation: &AnalysisPreparationLimits {
                max_picture_bytes: 3 * 1024 * 1024,
                max_request_bytes: 6 * 1024 * 1024,
                ..Default::default()
            },
            max_decoded_bytes: MEMORY,
            timeout,
            maximum_response_bytes: 4096,
        },
        "fixture-private-key",
    )
}
fn accepted(address: SocketAddr, model: &str) -> Value {
    let application = model.split('/').take(2).collect::<Vec<_>>().join("/");
    json!({"request_id":"req_1",
        "status_url":format!("http://{address}/{application}/requests/req_1/status"),
        "response_url":format!("http://{address}/{application}/requests/req_1")})
}
fn status(name: &str) -> (u16, Vec<u8>) {
    (
        200,
        json!({"status":name,"request_id":"req_1"})
            .to_string()
            .into_bytes(),
    )
}
fn two_people() -> Value {
    json!({"rle":["206 6 246 6 286 6","31 4 51 4"],"scores":[0.96,0.87],
        "boxes":[[0.2,0.4,0.15,0.3],[0.8,0.5,0.1,0.2]],
        "metadata":[{"index":1,"score":0.96},{"index":0,"score":0.87}],
        "boundingbox_frames_zip":null})
}
fn no_more(listener: &TcpListener) {
    assert!(
        matches!(listener.accept(), Err(error) if error.kind()==std::io::ErrorKind::WouldBlock)
    );
}
#[tokio::test]
async fn a_call_submits_once_polls_to_completion_and_reads_the_masks() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let model = "fal-ai/sam-3-1/image-rle";
    let mut prepared = prepare(address, model, 8, Duration::from_secs(5)).unwrap();
    let fixture = server(
        listener,
        vec![
            (200, accepted(address, model).to_string().into_bytes()),
            status("IN_QUEUE"),
            status("IN_PROGRESS"),
            status("COMPLETED"),
            (200, two_people().to_string().into_bytes()),
        ],
    );
    let raw = prepared.submit().await.unwrap();
    let checkpoint = prepared.checkpoint(&raw).unwrap();
    let result = prepared.resume(&checkpoint).await.unwrap();
    let answer = parse_segmentation(
        &serde_json::from_slice(&result.body).unwrap(),
        SegmentationPrompt::Person,
        40,
        20,
        8,
    )
    .unwrap();
    assert_eq!(answer.instances.len(), 2);
    assert_eq!(
        (answer.instances[0].index, answer.instances[0].score.get()),
        (1, 0.96)
    );
    assert_eq!(
        answer.instances[0]
            .runs
            .iter()
            .map(|run| run.length)
            .sum::<u32>(),
        18
    );
    assert_eq!(answer.instances[0].runs[0].start % 40, 5);
    let (listener, requests) = fixture.join().unwrap();
    assert_eq!(requests.len(), 5);
    let lines = requests
        .iter()
        .map(|raw| String::from_utf8_lossy(raw))
        .collect::<Vec<_>>();
    assert_eq!(
        lines.iter().filter(|raw| raw.starts_with("POST ")).count(),
        1
    );
    assert!(lines[0].starts_with("POST /fal-ai/sam-3-1/image-rle "));
    assert!(lines[0].contains("Key fixture-private-key"));
    let body = lines[0].split_once("\r\n\r\n").unwrap().1;
    let sent: Value = serde_json::from_str(body).unwrap();
    assert_eq!(sent["prompt"], "person");
    assert_eq!(sent["max_masks"], 8);
    assert!(
        sent["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );
    assert_eq!(
        lines.iter().filter(|raw| raw.starts_with("GET ")).count(),
        4
    );
    assert!(lines[4].starts_with("GET /fal-ai/sam-3-1/requests/req_1 "));
    no_more(&listener);
}
#[tokio::test]
async fn the_sam_3_model_goes_to_its_own_endpoint_and_an_empty_result_succeeds() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let model = "fal-ai/sam-3/image-rle";
    let mut prepared = prepare(address, model, 8, Duration::from_secs(3)).unwrap();
    let fixture = server(
        listener,
        vec![
            (200, accepted(address, model).to_string().into_bytes()),
            status("COMPLETED"),
            (200, br#"{"rle":[],"scores":[]}"#.to_vec()),
        ],
    );
    let raw = prepared.submit().await.unwrap();
    let checkpoint = prepared.checkpoint(&raw).unwrap();
    let response = prepared.resume(&checkpoint).await.unwrap();
    let masks = parse_segmentation(
        &serde_json::from_slice(&response.body).unwrap(),
        SegmentationPrompt::Person,
        40,
        20,
        8,
    )
    .unwrap();
    assert!(masks.instances.is_empty());
    let (listener, requests) = fixture.join().unwrap();
    assert!(requests[0].starts_with(b"POST /fal-ai/sam-3/image-rle "));
    no_more(&listener);
}
#[tokio::test]
async fn refused_and_unauthorized_submit_return_exact_raw_status_without_polling() {
    for status in [422, 401] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut prepared = prepare(
            listener.local_addr().unwrap(),
            "fal-ai/sam-3-1/image-rle",
            8,
            Duration::from_secs(3),
        )
        .unwrap();
        let body = br#"{"detail":"refused"}"#.to_vec();
        let fixture = server(listener, vec![(status, body.clone())]);
        let response = prepared.submit().await.unwrap();
        assert_eq!(response.status, status);
        assert_eq!(response.body, body);
        assert!(prepared.checkpoint(&response).is_err());
        let (listener, requests) = fixture.join().unwrap();
        assert_eq!(requests.len(), 1);
        no_more(&listener);
    }
}
#[tokio::test]
async fn a_request_fal_reports_failed_ends_polling_without_result_read() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let model = "fal-ai/sam-3-1/image-rle";
    let mut prepared = prepare(address, model, 8, Duration::from_secs(3)).unwrap();
    let fixture = server(
        listener,
        vec![
            (200, accepted(address, model).to_string().into_bytes()),
            status("IN_PROGRESS"),
            status("FAILED"),
        ],
    );
    let submitted = prepared.submit().await.unwrap();
    let response = prepared
        .resume(&prepared.checkpoint(&submitted).unwrap())
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap()["status"],
        "FAILED"
    );
    let (listener, requests) = fixture.join().unwrap();
    assert_eq!(requests.len(), 3);
    no_more(&listener);
}
#[tokio::test]
async fn queue_urls_that_name_another_host_or_request_are_refused_before_any_poll() {
    for hostile in [
        "https://evil.example/fal-ai/sam-3-1/requests/req_1/status",
        "https://queue.fal.run/fal-ai/sam-3-1/requests/another/status",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let model = "fal-ai/sam-3-1/image-rle";
        let mut prepared = prepare(address, model, 8, Duration::from_secs(3)).unwrap();
        let mut reply = accepted(address, model);
        reply["status_url"] = json!(hostile);
        let fixture = server(listener, vec![(200, reply.to_string().into_bytes())]);
        let response = prepared.submit().await.unwrap();
        assert!(prepared.checkpoint(&response).is_err());
        let (listener, requests) = fixture.join().unwrap();
        assert_eq!(requests.len(), 1);
        no_more(&listener);
    }
}
#[tokio::test]
async fn a_request_that_never_completes_ends_at_the_deadline_without_resubmission() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let model = "fal-ai/sam-3-1/image-rle";
    let mut prepared = prepare(address, model, 8, Duration::from_secs(2)).unwrap();
    let fixture = server(
        listener,
        vec![
            (200, accepted(address, model).to_string().into_bytes()),
            status("IN_PROGRESS"),
        ],
    );
    let raw = prepared.submit().await.unwrap();
    let checkpoint = prepared.checkpoint(&raw).unwrap();
    // Keep the bound listener alive without answering another poll. NativeHttp must expire.
    assert!(matches!(
        prepared.resume(&checkpoint).await,
        Err(RuntimeError::Expired)
    ));
    let (listener, requests) = fixture.join().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|raw| raw.starts_with(b"POST "))
            .count(),
        1
    );
    drop(listener);
}
#[test]
fn invalid_mask_count_is_refused_without_contacting_fal() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    for count in [0, 33] {
        assert!(matches!(
            prepare(
                listener.local_addr().unwrap(),
                "fal-ai/sam-3-1/image-rle",
                count,
                Duration::from_secs(3)
            ),
            Err(RuntimeError::InvalidRecord)
        ));
    }
    no_more(&listener);
}
#[test]
fn a_mask_that_does_not_fit_the_picture_is_a_bad_output() {
    assert!(matches!(
        parse_segmentation(
            &json!({"rle":["790 20"],"scores":[0.9]}),
            SegmentationPrompt::Person,
            40,
            20,
            8
        ),
        Err(RuntimeError::InvalidRecord)
    ));
}

fn parse_instances(
    value: &Value,
    width: u32,
    height: u32,
    maximum: u32,
) -> Result<Vec<df_providers::SegmentInstance>, RuntimeError> {
    let masks = parse_segmentation(value, SegmentationPrompt::Person, width, height, maximum)?;
    masks
        .instances
        .into_iter()
        .map(|mask| {
            Ok(df_providers::SegmentInstance {
                index: mask.index,
                score: mask.score.get(),
                mask: df_providers::MaskRows::from_runs(
                    width,
                    height,
                    mask.runs
                        .into_iter()
                        .map(|run| df_providers::MaskRun {
                            start: run.start,
                            length: run.length,
                        })
                        .collect(),
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            })
        })
        .collect()
}
#[test]
fn a_result_becomes_instances_with_fals_own_indices_and_empty_masks_dropped() {
    let value = json!({
        "rle": ["13 3 24 3", "", "1 4"],
        "scores": [0.96, 0.5, 0.87],
        "boxes": [],
        "metadata": [{"index": 2}, {"index": 0}, {"index": 1}],
    });
    let found = parse_instances(&value, 10, 5, 8).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!((found[0].index, found[0].score), (2, 0.96));
    assert_eq!((found[1].index, found[1].mask.pixel_count()), (1, 4));
    let none = json!({"rle": [], "scores": []});
    assert!(parse_instances(&none, 10, 5, 8).unwrap().is_empty());
}

#[test]
fn a_malformed_result_is_refused_whole() {
    for bad in [
        json!({"scores": [0.9]}),
        json!({"rle": ["1 1"]}),
        json!({"rle": ["1 1"], "scores": [0.9, 0.8]}),
        json!({"rle": ["1 1"], "scores": [1.5]}),
        json!({"rle": ["1 1"], "scores": ["x"]}),
        json!({"rle": ["1 99"], "scores": [0.9]}),
        json!({"rle": ["0 1"], "scores": [0.9]}),
        json!({"rle": [7], "scores": [0.9]}),
        json!({"rle": ["1 1", "3 1"], "scores": [0.9, 0.8]}),
        json!({"rle": ["1 1"], "scores": [0.9], "metadata": []}),
    ] {
        let maximum = if bad["rle"].as_array().is_some_and(|rows| rows.len() == 2) {
            1
        } else {
            8
        };
        assert_eq!(
            parse_instances(&bad, 10, 5, maximum),
            Err(RuntimeError::InvalidRecord),
            "{bad}"
        );
    }
}
