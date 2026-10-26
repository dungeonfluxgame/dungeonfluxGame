use super::*;
use crate::gameplay::{
    approach_model::ApproachModel,
    startup_configuration::ApproachSettings,
    storyteller::tests::{FakeEndpoint, Reply},
};
use serde_json::json;

#[tokio::test]
async fn pacing_jev_chooses_calm_and_tense_public_windows_under_closed_labels() {
    let endpoint = FakeEndpoint::start(["o1", "o3"].into_iter().map(|choice|
            Reply::Status(200, json!({"answers":{"q1":{"type":"choice","choice":choice,
                "confidence":0.9}},"model":"jev-1.13.0","usage":{"input_tokens":10,"output_tokens":1}})))
            .collect());
    let model = ApproachModel::configured(Some(ApproachSettings {
        credential: "jev-fixture-sentinel".into(),
        base_url: format!("http://{}", endpoint.address),
    }));
    let music = PlaceMusic::new(model, None, tokio::runtime::Handle::current(), || {});
    for (sequence, expected, mean) in [(1, PacingBand::Calm, 180), (2, PacingBand::Driving, 560)] {
        music.review_pacing(
            42,
            sequence,
            json!({"public_narrative":{"beat":"public"},
                "window_s":300,"intensity_mean_permille":mean}),
        );
        let mut decision = None;
        for _ in 0..100 {
            decision = music.pacing_result(42, sequence);
            if decision.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(decision, Some(Ok(expected)));
        assert_eq!(music.pacing_result(41, sequence), None);
        assert_eq!(
            music.pacing_result(42, sequence),
            None,
            "consume each decision once"
        );
    }
    let requests = endpoint.bodies();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["state"]["intensity_mean_permille"], 180);
    assert_eq!(requests[1]["state"]["intensity_mean_permille"], 560);
}
