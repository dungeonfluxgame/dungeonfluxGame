//! Preserved creator-classification policy/transport assertions against the unified owner.
use super::*;
use crate::{AccountHandle, AnalysisRoute, ApiEndpoint, ModelName, PrivateImageText};

fn creator(endpoint: &str, public: &str) -> CreatorClassification {
    CreatorClassification {
        route: AnalysisRoute {
            account: AccountHandle::new("fixture-jev").unwrap(),
            model: ModelName::new("jev-latest").unwrap(),
            endpoint: ApiEndpoint::new(endpoint).unwrap(),
            maximum_output_tokens: 512,
        },
        contract_revision: 1,
        public_appearance: PrivateImageText::new(public),
        general_prompt: None,
    }
}
fn answer(choice: &str) -> Value {
    json!({"answers":{"q1":{"type":"choice","choice":choice,"confidence":1.0}}})
}
#[test]
fn closed_categories_route_without_mature_content_leaking_into_restrained_mode() {
    use CreatorCategory as C;
    use CreatorRoute as R;
    let creator = creator(
        "http://127.0.0.1:1/v1/systemone",
        "adult fictional appearance",
    );
    let choices = [
        ("o1", C::General, R::General, R::General),
        ("o2", C::AdultNudity, R::Revise, R::Bfl),
        ("o3", C::AdultViolence, R::Revise, R::Bfl),
        ("o4", C::AdultNudityAndViolence, R::Revise, R::Bfl),
        ("o5", C::Unsupported, R::Revise, R::Revise),
        ("o6", C::Uncertain, R::Uncertain, R::Uncertain),
        ("o7", C::AdultNonsexualAnatomyDetail, R::Bfl, R::Bfl),
    ];
    for (choice, category, restrained, mature) in choices {
        for (rating, expected) in [
            (ImageContentRating::Restrained, restrained),
            (ImageContentRating::Mature, mature),
        ] {
            let actual = classify(&answer(choice), &creator, rating).unwrap();
            assert_eq!(actual.category, category);
            assert_eq!(actual.route, expected);
            assert_eq!(actual.contract_revision, 1);
        }
    }
    assert!(classify(&answer("o99"), &creator, ImageContentRating::Mature).is_err());
}

#[tokio::test]
async fn actual_jev_transport_preserves_public_state_and_uses_only_closed_choices() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    let public = "{\"menu_appearance\":\"An adult fictional dwarf in work clothes\",\"public_visual_details\":\"big tits\"}";
    for choice in ["o1", "o2", "o3", "o4", "o5", "o6", "o7", "o99"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let value = loop {
                let n = stream.read(&mut buffer).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(split) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..split]);
                    assert!(header.starts_with("POST /v1/systemone "));
                    let len = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap();
                    if bytes.len() >= split + 4 + len {
                        break serde_json::from_slice::<Value>(&bytes[split + 4..split + 4 + len])
                            .unwrap();
                    }
                }
            };
            assert_eq!(value["model"], "jev-latest");
            assert_eq!(value["state"]["public_appearance"], public);
            assert_eq!(
                value["state"]["classification_contract"],
                "creator-content-v3-jev-latest"
            );
            assert_eq!(
                value["questions"]["q1"]["criteria"]
                    .as_object()
                    .unwrap()
                    .len(),
                7
            );
            assert_eq!(
                value["questions"]["q1"]["instructions"],
                CLASSIFICATION_INSTRUCTIONS
            );
            for (index, criterion) in CLASSIFICATION_CRITERIA.iter().enumerate() {
                assert_eq!(
                    value["questions"]["q1"]["criteria"][format!("o{}", index + 1)],
                    *criterion
                );
            }
            assert!(value["state"].get("name").is_none());
            assert!(value["state"].get("backstory").is_none());
            let body = answer(choice).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let creator = creator(&format!("http://{address}/v1/systemone"), public);
        let response = PreparedClassification::prepare(
            &creator,
            "fixture-jev",
            Duration::from_secs(3),
            64 * 1024,
        )
        .unwrap()
        .execute()
        .await
        .unwrap();
        assert_eq!(response.status, 200);
        let value = parse_response(&response.body, 64 * 1024).unwrap();
        let actual = classify(&value, &creator, ImageContentRating::Mature);
        if choice == "o99" {
            assert!(actual.is_err());
        } else {
            assert_eq!(
                actual.unwrap(),
                classify(&answer(choice), &creator, ImageContentRating::Mature).unwrap()
            );
        }
        server.join().unwrap();
    }
}
