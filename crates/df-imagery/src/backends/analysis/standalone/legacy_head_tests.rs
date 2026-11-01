//! Exact parser/request cases relocated from the retired provider head locator.

use super::{ReadError, heads};
use crate::{AnalysisRoute, AnalysisTarget, PrivateImageText};
use heads::parse_heads;
use serde_json::Value;

fn targets() -> Vec<AnalysisTarget> {
    [
        ("speaker", "an innkeeper in a work apron"),
        ("hero-1", "the red-bearded dwarf fighter seen from behind"),
    ]
    .into_iter()
    .map(|(id, description)| AnalysisTarget {
        id: PrivateImageText::new(id),
        description: PrivateImageText::new(description),
    })
    .collect()
}
fn route() -> AnalysisRoute {
    AnalysisRoute {
        account: crate::AccountHandle::new("legacy-head-fixture").unwrap(),
        model: crate::ModelName::new("gpt-5.5").unwrap(),
        endpoint: crate::ApiEndpoint::new("http://127.0.0.1:9/chat").unwrap(),
        maximum_output_tokens: 2000,
    }
}
fn request_body(
    picture: &[u8],
    face: Option<&[u8]>,
    targets: &[AnalysisTarget],
) -> Result<Vec<u8>, ReadError> {
    heads::request_body(picture, face, targets, &route())
}
#[test]
fn heads_are_read_from_plain_or_fenced_json_and_unasked_or_repeated_entries_are_dropped() {
    let plain = parse_heads(
        r#"{"heads":[{"id":"speaker","left":0.328,"top":0.25,"right":0.396,"bottom":0.413},
               {"id":"hero-1","left":0.7,"top":0.2,"right":0.8,"bottom":0.4},
               {"id":"speaker","left":0.1,"top":0.1,"right":0.2,"bottom":0.2},
               {"id":"stranger","left":0.1,"top":0.1,"right":0.2,"bottom":0.2}]}"#,
        &targets(),
    )
    .unwrap();
    assert_eq!(plain.len(), 2);
    assert_eq!(plain[0].id.text(), "speaker");
    assert_eq!(plain[0].head.coordinates()[0], 0.328);
    assert_eq!(plain[1].id.text(), "hero-1");
    let fenced = parse_heads(
        "```json\n{\"heads\":[{\"id\":\"speaker\",\"left\":0.1,\"top\":0.2,\"right\":0.2,\"bottom\":0.35}]}\n```",
        &targets(),
    );
    assert_eq!(fenced.unwrap().len(), 1);
}

#[test]
fn answers_with_no_plausible_head_are_refused() {
    for bad in [
        "",
        "no head here",
        r#"{"heads":[]}"#,
        r#"{"heads":[{"id":"speaker","left":0.5,"top":0.2,"right":0.4,"bottom":0.3}]}"#,
        r#"{"heads":[{"id":"speaker","left":0.1,"top":0.1,"right":0.9,"bottom":0.4}]}"#,
        r#"{"heads":[{"id":"speaker","left":0.1,"top":0.1,"right":0.2,"bottom":0.9}]}"#,
        r#"{"heads":[{"id":"speaker","left":-0.1,"top":0.1,"right":0.2,"bottom":0.3}]}"#,
        r#"{"heads":[{"id":"speaker","left":0.1,"top":0.1,"right":1.4,"bottom":0.3}]}"#,
        r#"{"heads":[{"id":"speaker","left":"a","top":0.1,"right":0.2,"bottom":0.3}]}"#,
        r#"{"heads":[{"id":"speaker","left":0.1,"top":0.1,"right":0.2}]}"#,
        r#"{"left":0.1}"#,
    ] {
        assert_eq!(parse_heads(bad, &targets()), Err(ReadError::Shape), "{bad}");
    }
}

#[test]
fn the_request_carries_the_pictures_and_the_fixed_instruction_and_refuses_bad_input() {
    let body = request_body(b"\xff\xd8jpeg", Some(b"\xff\xd8face"), &targets()).unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["model"], "gpt-5.5");
    let content = &value["messages"][0]["content"];
    let text = content[0]["text"].as_str().unwrap();
    assert!(text.contains("speaker: an innkeeper in a work apron"));
    assert!(text.contains("hero-1: the red-bearded dwarf fighter"));
    assert!(text.contains("The second picture is the face of speaker"));
    for picture in [&content[1], &content[2]] {
        assert!(
            picture["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
    }
    let without = request_body(b"jpeg", None, &targets()[..1]).unwrap();
    let value: Value = serde_json::from_slice(&without).unwrap();
    assert_eq!(value["messages"][0]["content"].as_array().unwrap().len(), 2);
    assert_eq!(request_body(b"", None, &targets()), Err(ReadError::Shape));
    assert_eq!(request_body(b"jpeg", None, &[]), Err(ReadError::Shape));
    let long = "x".repeat(301);
    for (id, description) in [("speaker", long.as_str()), ("Bad Id", "x")] {
        assert_eq!(
            request_body(
                b"jpeg",
                None,
                &[AnalysisTarget {
                    id: PrivateImageText::new(id),
                    description: PrivateImageText::new(description)
                }]
            ),
            Err(ReadError::Shape)
        );
    }
}
